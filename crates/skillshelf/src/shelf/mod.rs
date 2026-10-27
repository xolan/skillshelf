//! The main window: navigation, the page lists, and the detail pane.

mod change;
mod discover;
mod installed;
mod licenses;
mod palette;
mod preferences;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, StyledExt, TitleBar, WindowExt,
    button::{Button, ButtonVariants},
    command::CommandState,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement,
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarMenu, SidebarMenuItem},
    spinner::Spinner,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use skillshelf_core::{
    Action as Step, AvailablePlugin, Env, FetchedSkill, Plugin, Provider, Skill, SkillHit,
    SkillUpdate, clean_staging, execute, scan_plugins, scan_skills,
};

use crate::settings::{self, GitHubAccess};
use crate::{assets, theme};

actions!(
    shelf,
    [
        Reload,
        FocusFilter,
        OpenPalette,
        SelectPrev,
        SelectNext,
        OpenSelected,
        UninstallSelected
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-r", Reload, Some("Shelf")),
        KeyBinding::new("ctrl-f", FocusFilter, Some("Shelf")),
        KeyBinding::new("ctrl-k", OpenPalette, Some("Shelf")),
        KeyBinding::new("up", SelectPrev, Some("Shelf")),
        KeyBinding::new("down", SelectNext, Some("Shelf")),
        KeyBinding::new("enter", OpenSelected, Some("Shelf")),
        // Not in the filter field, where delete edits the text.
        KeyBinding::new("delete", UninstallSelected, Some("Shelf")),
        // The filter field hands the arrows and enter to the list below it.
        KeyBinding::new("up", SelectPrev, Some("Shelf > Input")),
        KeyBinding::new("down", SelectNext, Some("Shelf > Input")),
        KeyBinding::new("enter", OpenSelected, Some("Shelf > Input")),
    ]);
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Skills,
    Plugins,
    FindSkills,
    FindPlugins,
    Settings,
    Licenses,
}

impl Page {
    /// The skill and plugin pages, which have nothing to show until an agent is turned on.
    fn needs_agents(self) -> bool {
        matches!(
            self,
            Page::Skills | Page::Plugins | Page::FindSkills | Page::FindPlugins
        )
    }

    fn placeholder(self) -> &'static str {
        match self {
            Page::Skills => "Filter skills",
            Page::Plugins => "Filter plugins",
            Page::FindSkills => "Search skills.sh",
            Page::FindPlugins => "Filter marketplace plugins",
            Page::Settings => "",
            Page::Licenses => "Filter licenses and crates",
        }
    }
}

#[derive(Clone, PartialEq)]
enum Selection {
    Skill(String),
    Plugin(String),
    /// A skills.sh result, by its id.
    Hit(String),
    /// A marketplace plugin, by its id.
    Offer(String),
    /// An asset or crate on the Licenses page, by index.
    License(usize),
}

pub struct Shelf {
    env: Option<Env>,
    page: Page,
    /// The agents skillshelf manages, as turned on in Settings; the others are left alone.
    agents: Vec<Provider>,
    /// Where each agent installed on this computer shows it is, once the environment is read.
    found: BTreeMap<Provider, PathBuf>,
    /// Skills of the managed agents.
    skills: Vec<Skill>,
    plugins: Vec<Plugin>,
    /// Providers whose plugin list could not be read.
    plugin_errors: Vec<String>,
    load_error: Option<SharedString>,
    loading: bool,
    busy: bool,
    selected: Option<Selection>,
    /// SKILL.md of the selected skill or download, without its frontmatter.
    skill_md: SharedString,
    /// Filters the lists; on Discover → Skills it is the skills.sh query.
    filter: Entity<InputState>,
    hits: Vec<SkillHit>,
    searching: bool,
    search_error: Option<SharedString>,
    search_task: Option<Task<()>>,
    /// The selected skills.sh result, downloaded for preview and install.
    fetched: Option<FetchedSkill>,
    fetching: bool,
    fetch_error: Option<SharedString>,
    fetch_task: Option<Task<()>>,
    /// Plugins the marketplaces offer; loaded the first time Discover → Plugins opens.
    offers: Option<Vec<AvailablePlugin>>,
    offer_errors: Vec<String>,
    loading_offers: bool,
    /// Update checks of installed skills, by name; a skill is checked when it is first selected.
    updates: HashMap<String, Result<Option<SkillUpdate>, SharedString>>,
    update_task: Option<Task<()>>,
    palette: Entity<CommandState>,
    /// Scrolls the selected row into view when the keyboard moves the selection.
    list_scroll: ScrollHandle,
    github_access: GitHubAccess,
    /// Where a personal access token is typed; cleared once saved.
    token_input: Entity<InputState>,
    has_token: bool,
    /// The result of the last GitHub access check, e.g. the hourly request limit.
    access_status: Option<Result<SharedString, SharedString>>,
    access_task: Option<Task<()>>,
    /// The widths dragged to: the sidebar's, and the details' beside a list and beside a license.
    sidebar_split: Entity<ResizableState>,
    detail_split: Entity<ResizableState>,
    license_split: Entity<ResizableState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shelf {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(Page::Skills.placeholder()));
        let token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("github_pat_…")
        });
        let subscription = cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.list_scroll.set_offset(Point::default());
                if this.page == Page::FindSkills {
                    this.search(cx);
                }
                cx.notify();
            }
        });
        let mut shelf = Shelf {
            env: None,
            page: Page::Skills,
            agents: settings::load_agents(),
            found: BTreeMap::new(),
            skills: Vec::new(),
            plugins: Vec::new(),
            plugin_errors: Vec::new(),
            load_error: None,
            loading: false,
            busy: false,
            selected: None,
            skill_md: SharedString::default(),
            filter,
            hits: Vec::new(),
            searching: false,
            search_error: None,
            search_task: None,
            fetched: None,
            fetching: false,
            fetch_error: None,
            fetch_task: None,
            offers: None,
            offer_errors: Vec::new(),
            loading_offers: false,
            updates: HashMap::new(),
            update_task: None,
            palette: cx.new(|cx| CommandState::new(window, cx)),
            list_scroll: ScrollHandle::new(),
            github_access: GitHubAccess::load(),
            token_input,
            has_token: settings::read_token().is_some(),
            access_status: None,
            access_task: None,
            sidebar_split: cx.new(|_| ResizableState::default()),
            detail_split: cx.new(|_| ResizableState::default()),
            license_split: cx.new(|_| ResizableState::default()),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
        };
        shelf.focus.focus(window, cx);
        shelf.reload(cx);
        shelf
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        let env = self.env.clone();
        let agents = self.agents.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let env = match env {
                        Some(env) => env,
                        None => {
                            let env = Env::detect()?;
                            clean_staging(&env);
                            env
                        }
                    };
                    let found = Provider::ALL
                        .into_iter()
                        .filter_map(|p| Some((p, env.locate(p)?)))
                        .collect();
                    let mut skills = installed::for_agents(scan_skills(&env)?, &agents);
                    let (mut plugins, errors) = scan_plugins(&env, &agents);
                    // Defaults go last, under their own heading.
                    skills.sort_by_key(|s| s.default);
                    plugins.sort_by_key(|p| p.is_default());
                    anyhow::Ok((env, found, skills, plugins, errors))
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match loaded {
                    Ok((env, found, skills, plugins, errors)) => {
                        this.env = Some(env);
                        this.found = found;
                        this.skills = skills;
                        this.plugins = plugins;
                        this.plugin_errors = errors;
                        this.load_error = None;
                    }
                    Err(e) => this.load_error = Some(format!("{e:#}").into()),
                }
                this.refresh_skill_md();
                this.check_update(cx);
                if this.page == Page::FindPlugins && this.offers.is_none() && !this.loading_offers {
                    this.load_offers(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reloads and forgets earlier update checks.
    fn reload_all(&mut self, cx: &mut Context<Self>) {
        self.updates.clear();
        self.update_task = None;
        self.reload(cx);
    }

    /// Runs planned steps in the background, then reloads.
    fn run(
        &mut self,
        steps: Vec<Step>,
        done: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(env) = self.env.clone() else {
            return;
        };
        if steps.is_empty() {
            return;
        }
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { execute(&env, &steps) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if let Some(done) = done {
                            window.push_notification(Notification::success(done), cx);
                        }
                    }
                    Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                }
                this.reload(cx);
                if this.offers.is_some() {
                    this.load_offers(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.select(None, cx);
        self.list_scroll.set_offset(Point::default());
        self.filter.update(cx, |filter, cx| {
            filter.set_value("", window, cx);
            filter.set_placeholder(page.placeholder(), window, cx);
        });
        if page == Page::FindPlugins && self.offers.is_none() {
            self.load_offers(cx);
        }
        if page == Page::Settings {
            self.check_access(cx);
        }
        if page == Page::FindSkills {
            self.hits.clear();
            self.search_error = None;
        }
        cx.notify();
    }

    fn select(&mut self, selection: Option<Selection>, cx: &mut Context<Self>) {
        if self.selected == selection {
            return;
        }
        self.selected = selection;
        self.set_fetched(None);
        self.fetch_task = None;
        self.fetching = false;
        self.fetch_error = None;
        self.update_task = None;
        if let Some(Selection::Hit(id)) = &self.selected
            && let Some(hit) = self.hits.iter().find(|h| &h.id == id).cloned()
        {
            self.fetch(hit, cx);
        }
        self.check_update(cx);
        self.refresh_skill_md();
        cx.notify();
    }

    /// Replaces the downloaded preview, removing the previous download from disk.
    fn set_fetched(&mut self, fetched: Option<FetchedSkill>) {
        if let Some(old) = self.fetched.take() {
            old.discard();
        }
        self.fetched = fetched;
    }

    fn refresh_skill_md(&mut self) {
        let text = match &self.selected {
            Some(Selection::Skill(name)) => self
                .skill(name)
                .and_then(Skill::skill_md)
                .and_then(|path| std::fs::read_to_string(path).ok()),
            Some(Selection::Hit(_)) => self.fetched.as_ref().map(|f| f.skill_md.clone()),
            _ => None,
        };
        self.skill_md = text
            .map(|t| strip_frontmatter(&t).into())
            .unwrap_or_default();
    }

    fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).value().trim().to_lowercase()
    }

    /// What each row of the current page's list selects, top to bottom.
    fn rows(&self, cx: &App) -> Vec<Selection> {
        if self.agents.is_empty() && self.page.needs_agents() {
            return Vec::new();
        }
        match self.page {
            Page::Skills => self
                .visible_skills(cx)
                .map(|s| Selection::Skill(s.name.clone()))
                .collect(),
            Page::Plugins => self
                .visible_plugins(cx)
                .map(|p| Selection::Plugin(p.id.clone()))
                .collect(),
            // A failed search shows its error, not the earlier results.
            Page::FindSkills if self.search_error.is_some() => Vec::new(),
            Page::FindSkills => self
                .hits
                .iter()
                .map(|h| Selection::Hit(h.id.clone()))
                .collect(),
            Page::FindPlugins => self
                .visible_offers(cx)
                .take(discover::MAX_OFFERS)
                .map(|o| Selection::Offer(o.id.clone()))
                .collect(),
            Page::Settings => Vec::new(),
            Page::Licenses => self
                .visible_licenses(cx)
                .map(|(ix, _)| Selection::License(ix))
                .collect(),
        }
    }

    /// Moves the selection `delta` rows down (up when negative) and scrolls to it.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = self
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| r == s));
        let ix = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        self.list_scroll.scroll_to_item(ix);
        self.select(Some(rows[ix].clone()), cx);
    }

    /// Enter: selects the first row, or runs the selected row's install or update.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.selected {
            None => self.move_selection(1, cx),
            Some(Selection::Skill(_)) => self.download_update(window, cx),
            Some(Selection::Hit(_)) => self.install_hit(window, cx),
            Some(Selection::Offer(_)) => self.install_offer(window, cx),
            Some(Selection::Plugin(_) | Selection::License(_)) => {}
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_theme = cx.theme().theme_name().clone();
        let working = self.loading || self.busy || self.fetching || self.loading_offers;
        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .justify_between()
                .child(
                    h_flex().gap_2().child(img(assets::LOGO).size_4()).child(
                        h_flex()
                            .font_semibold()
                            .child("skill")
                            .child(div().text_color(cx.theme().primary).child("shelf")),
                    ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .when(working, |this| {
                            this.child(Spinner::new().small().color(cx.theme().muted_foreground))
                        })
                        .child(
                            Button::new("reload")
                                .ghost()
                                .small()
                                .icon(IconName::RefreshCw)
                                .tooltip("Reload (Ctrl+R)")
                                .on_click(cx.listener(|this, _, _, cx| this.reload_all(cx))),
                        )
                        .child(
                            Button::new("theme")
                                .ghost()
                                .small()
                                .icon(IconName::Palette)
                                .tooltip("Theme")
                                .dropdown_menu(move |menu, _, cx| {
                                    let mut menu = menu.scrollable(true).max_h(px(420.));
                                    for (name, _) in theme::names(cx) {
                                        let checked = name == active_theme;
                                        let pick = name.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(name)
                                                .checked(checked)
                                                .on_click(move |_, _, cx| theme::select(&pick, cx)),
                                        );
                                    }
                                    menu
                                }),
                        ),
                ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |page: Page, label: &'static str, icon: Icon, count: Option<usize>| {
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(self.page == page)
                .when_some(count, |item, count| {
                    item.suffix(move |_, cx| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count.to_string())
                    })
                })
                .on_click(cx.listener(move |this, _, window, cx| this.open_page(page, window, cx)))
        };
        // Fills its resizable panel, whose handle draws the divider.
        Sidebar::new("navigation")
            .w_full()
            .border_r_0()
            .line_height(icon_line())
            .child(
                SidebarGroup::new("Installed").child(SidebarMenu::new().children([
                    item(
                        Page::Skills,
                        "Skills",
                        IconName::BookOpen.into(),
                        Some(self.skills.len()),
                    ),
                    item(
                        Page::Plugins,
                        "Plugins",
                        Icon::new(Lucide::Puzzle),
                        Some(self.plugins.len()),
                    ),
                ])),
            )
            .child(
                SidebarGroup::new("Discover").child(SidebarMenu::new().children([
                    item(Page::FindSkills, "Skills", IconName::Search.into(), None),
                    item(Page::FindPlugins, "Plugins", Icon::new(Lucide::Store), None),
                ])),
            )
            .footer(
                SidebarFooter::new()
                    .selected(self.page == Page::Settings)
                    .child(
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .child(IconName::Settings)
                            .child("Settings"),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.open_page(Page::Settings, window, cx)
                        }),
                    ),
            )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.agents.is_empty() && self.page.needs_agents() {
            return self.no_agents(cx);
        }
        match self.page {
            Page::Skills => self.render_skills(cx),
            Page::Plugins => self.render_plugins(cx),
            Page::FindSkills => self.render_find_skills(cx),
            Page::FindPlugins => self.render_find_plugins(cx),
            Page::Settings => self.render_settings(cx),
            Page::Licenses => self.render_licenses(cx),
        }
    }

    fn no_agents(&self, cx: &mut Context<Self>) -> AnyElement {
        empty_state(
            IconName::Settings,
            "No agents turned on",
            "Turn on the agents you use in Settings. skillshelf lists and changes skills and \
             plugins for those only.",
            cx,
        )
        .child(
            Button::new("open-settings")
                .small()
                .primary()
                .label("Open Settings")
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_page(Page::Settings, window, cx)),
                ),
        )
        .into_any_element()
    }

    /// Column titles for the managed agents, after an "All" column when there are several.
    fn agent_columns(&self) -> Vec<&'static str> {
        let all = (self.agents.len() > 1).then_some("All");
        all.into_iter()
            .chain(self.agents.iter().map(|p| p.label()))
            .collect()
    }

    fn page_header(
        &self,
        title: &str,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .px_4()
            .py_3()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_lg().font_semibold().child(title.to_owned()))
            .when_some(count, |this, count| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(count.to_string()),
                )
            })
            .child(div().flex_1())
            .child(
                div().w(px(300.)).child(
                    Input::new(&self.filter)
                        .small()
                        .prefix(IconName::Search)
                        .aria_label(self.page.placeholder())
                        .cleanable(true),
                ),
            )
    }

    /// Column titles over the per-row controls.
    fn column_header(&self, columns: &[&'static str], cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_4()
            .py_1()
            .gap_3()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().child("Name"))
            .children(
                columns
                    .iter()
                    .map(|label| div().w(px(COLUMN)).text_center().child(*label)),
            )
    }

    fn switch(
        &self,
        id: String,
        checked: bool,
        disabled: bool,
        label: &'static str,
        on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        cell()
            .child(
                Switch::new(SharedString::from(id))
                    .small()
                    .checked(checked)
                    .disabled(disabled || self.busy)
                    .tooltip(label)
                    .on_change(on_change),
            )
            .into_any_element()
    }

    /// One list row: name, tag and description on the left, `controls` in columns on the right.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        id: String,
        selected: bool,
        name: String,
        tag: Option<&str>,
        description: String,
        controls: Vec<AnyElement>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id(SharedString::from(id))
            .px_4()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(theme.border)
            .when(selected, |row| row.bg(theme.list_active))
            .when(!selected, |row| {
                row.hover(|style| style.bg(theme.list_hover))
            })
            .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .on_click(on_click)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_medium().child(name))
                            .when_some(tag, |row, tag| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(tag.to_owned()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(description),
                    ),
            )
            .children(controls)
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.selected.as_ref()? {
            Selection::Skill(name) => self.skill_detail(self.skill(name)?, cx),
            Selection::Plugin(id) => self.plugin_detail(self.plugin(id)?, cx),
            Selection::Hit(id) => self.hit_detail(self.hits.iter().find(|h| &h.id == id)?, cx),
            Selection::Offer(id) => self.offer_detail(self.offer(id)?, cx),
            Selection::License(ix) => self.license_detail(*ix, cx)?,
        })
    }

    /// The page beside the selected item's details, split by a divider that drags.
    fn render_main(&self, content: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        let detail = self.render_detail(cx);
        // License texts are long lines of prose: they get more room, and a width of their own.
        let (id, state, width) = match self.page {
            Page::Licenses => ("license-split", &self.license_split, px(640.)),
            _ => ("detail-split", &self.detail_split, px(400.)),
        };
        h_resizable(id)
            .with_state(state)
            .child(
                resizable_panel()
                    .size_range(px(240.)..Pixels::MAX)
                    .child(div().size_full().child(content)),
            )
            .child(
                resizable_panel()
                    .visible(detail.is_some())
                    .size(width)
                    .size_range(px(280.)..px(900.))
                    .fixed()
                    .flex_none()
                    .children(detail),
            )
    }

    fn detail_header(
        &self,
        title: String,
        subtitle: Option<String>,
        action: impl Into<Option<Button>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .px_4()
            .py_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_lg().font_semibold().truncate().child(title))
                    .when_some(subtitle, |this, subtitle| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .truncate()
                                .child(subtitle),
                        )
                    }),
            )
            .children(action.into().map(|a| a.small()))
    }
}

impl Render for Shelf {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.load_error {
            Some(error) => empty_state(
                IconName::CircleAlert,
                "Could not read your setup",
                error,
                cx,
            )
            .into_any_element(),
            None => self.render_page(cx),
        };
        v_flex()
            .size_full()
            .key_context("Shelf")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload_all(cx)))
            .on_action(
                cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &UninstallSelected, window, cx| {
                this.confirm_uninstall(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenSelected, window, cx| this.open_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter
                    .update(cx, |filter, cx| filter.focus(window, cx));
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("sidebar-split")
                        .with_state(&self.sidebar_split)
                        .child(
                            resizable_panel()
                                .size(px(210.))
                                .size_range(px(160.)..px(360.))
                                .fixed()
                                .flex_none()
                                .child(self.render_sidebar(cx)),
                        )
                        .child(resizable_panel().child(self.render_main(content, cx))),
                ),
            )
    }
}

/// Width of a control column on the right of a row.
const COLUMN: f32 = 64.;

fn cell() -> Div {
    div().w(px(COLUMN)).flex().justify_center()
}

/// Line height for 14 px text beside an icon. An even 20 px puts Geist's cap height on the middle
/// of the line, where the icon is centred; with gpui's default (1.618, so 23 px) the text sits
/// half a pixel to a pixel above the icon.
fn icon_line() -> Rems {
    rems(1.25)
}

/// An icon beside text that may wrap, centred on the first line.
fn icon_row(icon: impl IntoElement, text: impl IntoElement) -> Div {
    h_flex()
        .gap_2()
        .items_start()
        .line_height(icon_line())
        .child(h_flex().flex_shrink_0().h(icon_line()).child(icon))
        .child(div().flex_1().min_w_0().child(text))
}

/// A notice row above a list, e.g. a provider whose tool failed.
fn warning(text: impl Into<SharedString>, cx: &mut Context<Shelf>) -> Div {
    icon_row(IconName::TriangleAlert, text.into())
        .px_4()
        .py_2()
        .text_sm()
        .bg(cx.theme().warning.opacity(0.12))
}

fn list_body(
    id: &'static str,
    rows: Vec<AnyElement>,
    scroll: &ScrollHandle,
    cx: &mut Context<Shelf>,
) -> AnyElement {
    if rows.is_empty() {
        return empty_state(IconName::Inbox, "Nothing here", "No matches.", cx).into_any_element();
    }
    // The rows are the scroll area's direct children, so `scroll_to_item` can find them.
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .child(
            v_flex()
                .id(id)
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .children(rows),
        )
        .vertical_scrollbar(scroll)
        .into_any_element()
}

fn empty_state(icon: IconName, title: &str, body: &str, cx: &mut Context<Shelf>) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .px_8()
        .text_color(cx.theme().muted_foreground)
        .child(div().text_2xl().child(icon))
        .child(
            div()
                .font_medium()
                .text_color(cx.theme().foreground)
                .child(title.to_owned()),
        )
        .child(div().text_sm().text_center().child(body.to_owned()))
}

fn fact(label: &str, value: String, cx: &mut Context<Shelf>) -> Div {
    h_flex()
        .gap_2()
        .text_sm()
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().min_w_0().child(value))
}

fn divider(cx: &mut Context<Shelf>) -> Div {
    div().h(px(1.)).bg(cx.theme().border)
}

fn matches(filter: &str, text: &[&str]) -> bool {
    filter.is_empty() || text.iter().any(|t| t.to_lowercase().contains(filter))
}

fn strip_frontmatter(text: &str) -> String {
    text.strip_prefix("---")
        .and_then(|rest| rest.split_once("\n---"))
        .map(|(_, body)| body.trim_start_matches(['-', '\n', '\r']).to_owned())
        .unwrap_or_else(|| text.to_owned())
}

/// How long to wait after the last keystroke before searching skills.sh.
const SEARCH_DELAY: Duration = Duration::from_millis(300);
