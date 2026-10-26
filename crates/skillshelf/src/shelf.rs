//! The main window: navigation, the installed skills and plugins, and the detail pane.

use std::collections::BTreeSet;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, TitleBar, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    scroll::ScrollableElement,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
    spinner::Spinner,
    switch::Switch,
    text::TextView,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use skillshelf_core::{
    Action as Step, Env, Plugin, Provider, Skill, execute, scan_plugins, scan_skills,
    toggle_plugin, toggle_skill, uninstall_plugin, uninstall_skill,
};

use crate::theme;

actions!(shelf, [Reload, FocusFilter]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-r", Reload, Some("Shelf")),
        KeyBinding::new("ctrl-f", FocusFilter, Some("Shelf")),
    ]);
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Skills,
    Plugins,
    FindSkills,
    FindPlugins,
}

#[derive(Clone, PartialEq)]
enum Selection {
    Skill(String),
    Plugin(String),
}

pub struct Shelf {
    env: Option<Env>,
    page: Page,
    skills: Vec<Skill>,
    plugins: Vec<Plugin>,
    /// Providers whose plugin list could not be read.
    plugin_errors: Vec<String>,
    load_error: Option<SharedString>,
    loading: bool,
    busy: bool,
    selected: Option<Selection>,
    /// SKILL.md of the selected skill, without its frontmatter.
    skill_md: SharedString,
    filter: Entity<InputState>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shelf {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let subscription = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut shelf = Shelf {
            env: None,
            page: Page::Skills,
            skills: Vec::new(),
            plugins: Vec::new(),
            plugin_errors: Vec::new(),
            load_error: None,
            loading: false,
            busy: false,
            selected: None,
            skill_md: SharedString::default(),
            filter,
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
        };
        shelf.reload(cx);
        shelf
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        let env = self.env.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let env = match env {
                        Some(env) => env,
                        None => Env::detect()?,
                    };
                    let skills = scan_skills(&env)?;
                    let (plugins, errors) = scan_plugins(&env);
                    anyhow::Ok((env, skills, plugins, errors))
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match loaded {
                    Ok((env, skills, plugins, errors)) => {
                        this.env = Some(env);
                        this.skills = skills;
                        this.plugins = plugins;
                        this.plugin_errors = errors;
                        this.load_error = None;
                    }
                    Err(e) => this.load_error = Some(format!("{e:#}").into()),
                }
                this.refresh_detail();
                cx.notify();
            })
            .ok();
        })
        .detach();
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
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selected = Some(selection);
        self.refresh_detail();
        cx.notify();
    }

    fn refresh_detail(&mut self) {
        self.skill_md = match &self.selected {
            Some(Selection::Skill(name)) => self
                .skill(name)
                .and_then(Skill::skill_md)
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|text| strip_frontmatter(&text).into())
                .unwrap_or_default(),
            _ => SharedString::default(),
        };
    }

    fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    fn plugin(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    fn set_skill_enabled(
        &mut self,
        name: &str,
        providers: &[Provider],
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(env), Some(skill)) = (&self.env, self.skill(name)) else {
            return;
        };
        let steps = providers
            .iter()
            .flat_map(|&p| toggle_skill(env, skill, p, enabled))
            .collect();
        self.run(steps, None, window, cx);
    }

    fn set_plugin_enabled(
        &mut self,
        id: &str,
        providers: &[Provider],
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(plugin) = self.plugin(id) else {
            return;
        };
        let steps = providers
            .iter()
            .flat_map(|&p| toggle_plugin(plugin, p, enabled))
            .collect();
        self.run(steps, None, window, cx);
    }

    fn confirm_uninstall(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(env) = self.env.clone() else {
            return;
        };
        let target = match &self.selected {
            Some(Selection::Skill(name)) => self.skill(name).cloned().map(Target::Skill),
            Some(Selection::Plugin(id)) => self.plugin(id).cloned().map(Target::Plugin),
            None => None,
        };
        let Some(target) = target else {
            return;
        };
        let name: SharedString = target.name().into();
        let form = cx.new(|_| UninstallForm::new(env, target));
        let shelf = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (form, shelf, name) = (form.clone(), shelf.clone(), name.clone());
            alert
                .title(format!("Uninstall “{name}”?"))
                .description(form.clone())
                .width(px(620.))
                .show_cancel(true)
                .ok_text("Uninstall")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let steps = form.read(cx).steps();
                    if steps.is_empty() {
                        return false;
                    }
                    let done = Some(format!("Uninstalled {name}").into());
                    shelf.update(cx, |shelf, cx| shelf.run(steps, done, window, cx));
                    true
                })
        });
    }

    fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_lowercase()
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_theme = cx.theme().theme_name().clone();
        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .justify_between()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_color(cx.theme().primary)
                                .child(Lucide::LibraryBig),
                        )
                        .child(div().font_semibold().child("skillshelf")),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .when(self.loading || self.busy, |this| {
                            this.child(Spinner::new().small().color(cx.theme().muted_foreground))
                        })
                        .child(
                            Button::new("reload")
                                .ghost()
                                .small()
                                .icon(IconName::RefreshCw)
                                .tooltip("Reload (Ctrl+R)")
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
        };
        Sidebar::new("navigation")
            .w(px(210.))
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
    }

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::Skills => self.render_skills(cx).into_any_element(),
            Page::Plugins => self.render_plugins(cx).into_any_element(),
            Page::FindSkills | Page::FindPlugins => empty_state(
                IconName::Search,
                "Discover is not built yet",
                "Search for skills on skills.sh and plugins in your marketplaces.",
                cx,
            )
            .into_any_element(),
        }
    }

    fn page_header(&self, title: &str, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_4()
            .py_3()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_lg().font_semibold().child(title.to_owned()))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(count.to_string()),
            )
            .child(div().flex_1())
            .child(
                div().w(px(260.)).child(
                    Input::new(&self.filter)
                        .small()
                        .prefix(IconName::Search)
                        .cleanable(true),
                ),
            )
    }

    fn column_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                ["All", "Claude", "Codex"].map(|label| div().w(px(56.)).text_center().child(label)),
            )
    }

    fn render_skills(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter_text(cx);
        let rows: Vec<_> = self
            .skills
            .iter()
            .filter(|s| matches(&filter, &[&s.name, &s.description]))
            .map(|s| self.skill_row(s, cx))
            .collect();
        let count = rows.len();
        v_flex()
            .size_full()
            .child(self.page_header("Skills", count, cx))
            .child(self.column_header(cx))
            .child(list_body("skill-list", rows, cx))
    }

    fn skill_row(&self, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let name = skill.name.clone();
        let togglable: Vec<Provider> = Provider::ALL
            .into_iter()
            .filter(|&p| {
                skill.enabled(p).is_some() || (p == Provider::Claude && skill.shared.is_some())
            })
            .collect();
        let all_on = togglable.iter().all(|&p| skill.enabled(p) == Some(true));
        let mut switches = vec![self.switch(
            format!("skill-all-{name}"),
            all_on,
            togglable.is_empty(),
            "All providers",
            {
                let name = name.clone();
                let togglable = togglable.clone();
                cx.listener(move |this, on: &bool, window, cx| {
                    this.set_skill_enabled(&name, &togglable, *on, window, cx)
                })
            },
        )];
        for provider in Provider::ALL {
            let name = name.clone();
            switches.push(self.switch(
                format!("skill-{}-{name}", provider.cli()),
                skill.enabled(provider) == Some(true),
                !togglable.contains(&provider),
                provider.label(),
                cx.listener(move |this, on: &bool, window, cx| {
                    this.set_skill_enabled(&name, &[provider], *on, window, cx)
                }),
            ));
        }
        let selected = self.selected == Some(Selection::Skill(name.clone()));
        let unmanaged = skill.shared.is_none();
        self.row(
            format!("skill-{name}"),
            selected,
            skill.name.clone(),
            unmanaged.then_some("unmanaged"),
            skill.description.clone(),
            switches,
            cx.listener(move |this, _, _, cx| this.select(Selection::Skill(name.clone()), cx)),
            cx,
        )
    }

    fn render_plugins(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter_text(cx);
        let rows: Vec<_> = self
            .plugins
            .iter()
            .filter(|p| matches(&filter, &[&p.id, &p.description]))
            .map(|p| self.plugin_row(p, cx))
            .collect();
        let count = rows.len();
        v_flex()
            .size_full()
            .child(self.page_header("Plugins", count, cx))
            .children(self.plugin_errors.iter().map(|error| {
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .text_sm()
                    .bg(cx.theme().warning.opacity(0.12))
                    .child(IconName::TriangleAlert)
                    .child(error.clone())
            }))
            .child(self.column_header(cx))
            .child(list_body("plugin-list", rows, cx))
    }

    fn plugin_row(&self, plugin: &Plugin, cx: &mut Context<Self>) -> AnyElement {
        let id = plugin.id.clone();
        let installed: Vec<Provider> = plugin.installs.keys().copied().collect();
        let all_on = plugin.installs.values().all(|i| i.enabled);
        let mut switches = vec![self.switch(
            format!("plugin-all-{id}"),
            all_on,
            false,
            "All providers",
            {
                let id = id.clone();
                let installed = installed.clone();
                cx.listener(move |this, on: &bool, window, cx| {
                    this.set_plugin_enabled(&id, &installed, *on, window, cx)
                })
            },
        )];
        for provider in Provider::ALL {
            let id = id.clone();
            let install = plugin.installs.get(&provider);
            switches.push(self.switch(
                format!("plugin-{}-{id}", provider.cli()),
                install.is_some_and(|i| i.enabled),
                install.is_none(),
                provider.label(),
                cx.listener(move |this, on: &bool, window, cx| {
                    this.set_plugin_enabled(&id, &[provider], *on, window, cx)
                }),
            ));
        }
        let selected = self.selected == Some(Selection::Plugin(id.clone()));
        self.row(
            format!("plugin-{id}"),
            selected,
            plugin.name.clone(),
            Some(plugin.marketplace.as_str()).filter(|m| !m.is_empty()),
            plugin.description.clone(),
            switches,
            cx.listener(move |this, _, _, cx| this.select(Selection::Plugin(id.clone()), cx)),
            cx,
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
        div()
            .w(px(56.))
            .flex()
            .justify_center()
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

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        id: String,
        selected: bool,
        name: String,
        tag: Option<&str>,
        description: String,
        switches: Vec<AnyElement>,
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
            .children(switches)
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let body = match self.selected.as_ref()? {
            Selection::Skill(name) => self.skill_detail(self.skill(name)?, cx),
            Selection::Plugin(id) => self.plugin_detail(self.plugin(id)?, cx),
        };
        Some(
            v_flex()
                .w(px(400.))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(cx.theme().border)
                .child(body),
        )
    }

    fn detail_header(
        &self,
        title: String,
        subtitle: Option<String>,
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
            .child(
                Button::new("uninstall")
                    .small()
                    .danger()
                    .icon(Lucide::Trash)
                    .label("Uninstall…")
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.confirm_uninstall(window, cx)),
                    ),
            )
    }

    fn skill_detail(&self, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let Some(env) = &self.env else {
            return div().into_any_element();
        };
        let locations: Vec<_> = skill
            .installs
            .iter()
            .map(|install| {
                let kind = match std::fs::read_link(&install.path) {
                    Ok(target) => format!("link to {}", target.display()),
                    Err(_) if Some(&install.path) == skill.shared.as_ref() => "shared copy".into(),
                    Err(_) => "folder".to_owned(),
                };
                let state = if install.enabled { "on" } else { "off" };
                let place = env.tilde(&install.path);
                fact(
                    install.provider.label(),
                    format!("{place} ({kind}, {state})"),
                    cx,
                )
            })
            .collect();
        v_flex()
            .size_full()
            .child(
                self.detail_header(
                    skill.title.clone(),
                    skill.source.clone().or(skill
                        .shared
                        .is_none()
                        .then(|| "Not managed by skillshelf".into())),
                    cx,
                ),
            )
            .child(
                div()
                    .id("skill-detail")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .p_4()
                            .gap_3()
                            .child(skill.description.clone())
                            .child(v_flex().gap_1().children(locations))
                            .child(div().h(px(1.)).bg(cx.theme().border))
                            .child(
                                TextView::markdown("skill-md", self.skill_md.clone())
                                    .selectable(true),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn plugin_detail(&self, plugin: &Plugin, cx: &mut Context<Self>) -> AnyElement {
        let Some(env) = &self.env else {
            return div().into_any_element();
        };
        let installs: Vec<_> = plugin
            .installs
            .iter()
            .map(|(provider, install)| {
                let state = if install.enabled { "on" } else { "off" };
                let place = install
                    .path
                    .as_deref()
                    .map(|p| format!(", {}", env.tilde(p)))
                    .unwrap_or_default();
                let summary = format!("{} ({state}{place})", install.version);
                fact(provider.label(), summary, cx)
            })
            .collect();
        v_flex()
            .size_full()
            .child(self.detail_header(plugin.name.clone(), Some(plugin.marketplace.clone()), cx))
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .child(plugin.description.clone())
                    .child(v_flex().gap_1().children(installs)),
            )
            .into_any_element()
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
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter
                    .update(cx, |filter, cx| filter.focus(window, cx));
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(div().flex_1().min_w_0().h_full().child(content))
                    .children(self.render_detail(cx)),
            )
    }
}

/// What a dialog uninstalls.
enum Target {
    Skill(Skill),
    Plugin(Plugin),
}

impl Target {
    fn name(&self) -> &str {
        match self {
            Target::Skill(skill) => &skill.name,
            Target::Plugin(plugin) => &plugin.id,
        }
    }

    fn providers(&self) -> BTreeSet<Provider> {
        match self {
            Target::Skill(skill) => skill.installs.iter().map(|i| i.provider).collect(),
            Target::Plugin(plugin) => plugin.installs.keys().copied().collect(),
        }
    }
}

/// The body of the uninstall dialog: which providers to remove it from, and every step that
/// will run.
struct UninstallForm {
    env: Env,
    target: Target,
    providers: BTreeSet<Provider>,
    delete_data: bool,
}

impl UninstallForm {
    fn new(env: Env, target: Target) -> Self {
        let providers = target.providers();
        UninstallForm {
            env,
            target,
            providers,
            delete_data: true,
        }
    }

    fn steps(&self) -> Vec<Step> {
        let providers: Vec<Provider> = self.providers.iter().copied().collect();
        match &self.target {
            Target::Skill(skill) => uninstall_skill(skill, &providers),
            Target::Plugin(plugin) => uninstall_plugin(plugin, &providers, self.delete_data),
        }
    }
}

impl Render for UninstallForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_claude_plugin =
            matches!(&self.target, Target::Plugin(p) if p.installs.contains_key(&Provider::Claude));
        let steps = self.steps();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_4()
                    .children(self.target.providers().into_iter().map(|provider| {
                        Checkbox::new(SharedString::from(format!("uninstall-{}", provider.cli())))
                            .label(provider.label())
                            .checked(self.providers.contains(&provider))
                            .on_change(cx.listener(move |this, on: &bool, _, cx| {
                                if *on {
                                    this.providers.insert(provider);
                                } else {
                                    this.providers.remove(&provider);
                                }
                                cx.notify();
                            }))
                    })),
            )
            .when(has_claude_plugin, |this| {
                this.child(
                    Checkbox::new("uninstall-data")
                        .label("Also delete the plugin's saved data (Claude)")
                        .checked(self.delete_data)
                        .on_change(cx.listener(|this, on: &bool, _, cx| {
                            this.delete_data = *on;
                            cx.notify();
                        })),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .when(steps.is_empty(), |this| this.child("Nothing to do."))
                    .children(steps.iter().map(|step| step.describe(&self.env))),
            )
    }
}

fn list_body(id: &'static str, rows: Vec<AnyElement>, cx: &mut Context<Shelf>) -> AnyElement {
    if rows.is_empty() {
        return empty_state(IconName::Inbox, "Nothing here", "No matches.", cx).into_any_element();
    }
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(v_flex().children(rows))
        .into_any_element()
}

fn empty_state(icon: IconName, title: &str, body: &str, cx: &mut Context<Shelf>) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(div().text_2xl().child(icon))
        .child(
            div()
                .font_medium()
                .text_color(cx.theme().foreground)
                .child(title.to_owned()),
        )
        .child(div().text_sm().child(body.to_owned()))
}

fn fact(label: &str, value: String, cx: &mut Context<Shelf>) -> Div {
    h_flex()
        .gap_2()
        .text_sm()
        .child(
            div()
                .w(px(56.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().min_w_0().child(value))
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
