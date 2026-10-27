//! Installed skills and plugins: lists with per-provider switches, details, uninstall.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt, button::Button, button::ButtonVariants,
    h_flex, scroll::ScrollableElement, spinner::Spinner, text::TextView, v_flex,
};
use gpui_kit::*;
use skillshelf_core::{
    Plugin, Provider, Skill, check_skill_update, fetch_update, toggle_plugin, toggle_skill,
};

use super::change::{Change, open_change_dialog};
use super::{SEARCH_DELAY, Selection, Shelf, cell, divider, fact, list_body, matches, warning};
use crate::settings::GitHubAccess;

/// The skills to list when skillshelf manages only `agents`: those an agent has, or Claude can
/// link in from the shared folder. Each keeps its installs for those agents, and the links and
/// copies of its shared folder that other agents have, so they go with the folder on uninstall.
pub(super) fn for_agents(skills: Vec<Skill>, agents: &[Provider]) -> Vec<Skill> {
    skills
        .into_iter()
        .filter_map(|mut skill| {
            skill
                .installs
                .retain(|i| agents.contains(&i.provider) || i.shared);
            let listed = skill.installs.iter().any(|i| agents.contains(&i.provider))
                || (agents.contains(&Provider::Claude) && skill.shared.is_some());
            listed.then_some(skill)
        })
        .collect()
}

/// Puts the "Default installs" heading on top of the first default row. It goes inside that row, so each
/// row stays one child of the list and `scroll_to_item` keeps finding it.
fn with_default_heading(rows: &mut [AnyElement], first: Option<usize>, cx: &mut Context<Shelf>) {
    let Some(row) = first.and_then(|ix| rows.get_mut(ix)) else {
        return;
    };
    let theme = cx.theme();
    // A tinted band between rule lines, so it reads as a section break and not as another row.
    let heading = h_flex()
        .px_4()
        .py_2()
        .gap_2()
        .bg(theme.primary.opacity(0.12))
        .border_t_2()
        .border_b_1()
        .border_color(theme.primary.opacity(0.5))
        .child(Icon::new(Lucide::Lock).small().text_color(theme.primary))
        .child(div().font_semibold().child("Default installs"))
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .truncate()
                .child("Come with the agent; cannot be removed or turned off"),
        );
    let taken = std::mem::replace(row, div().into_any_element());
    *row = v_flex().child(heading).child(taken).into_any_element();
}

impl Shelf {
    pub(super) fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub(super) fn plugin(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub(super) fn visible_skills(&self, cx: &App) -> impl Iterator<Item = &Skill> {
        let filter = self.filter_text(cx);
        self.skills
            .iter()
            .filter(move |s| matches(&filter, &[&s.name, &s.description]))
    }

    pub(super) fn visible_plugins(&self, cx: &App) -> impl Iterator<Item = &Plugin> {
        let filter = self.filter_text(cx);
        self.plugins
            .iter()
            .filter(move |p| matches(&filter, &[&p.id, &p.description]))
    }

    fn has_update(&self, name: &str) -> bool {
        matches!(self.updates.get(name), Some(Ok(Some(_))))
    }

    /// Asks GitHub whether the selected skill has a newer version, once per skill and only for
    /// skills installed from GitHub.
    pub(super) fn check_update(&mut self, cx: &mut Context<Self>) {
        let Some(Selection::Skill(name)) = self.selected.clone() else {
            return;
        };
        let (Some(env), Some(skill)) = (self.env.clone(), self.skill(&name)) else {
            return;
        };
        if skill.source.is_none() || skill.shared.is_none() || self.updates.contains_key(&name) {
            return;
        }
        // Waits like the search does, so arrowing through the list sends no requests.
        self.update_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let result = {
                let name = name.clone();
                cx.background_spawn(async move {
                    let token = GitHubAccess::load().token(&env).ok().flatten();
                    check_skill_update(&env, &name, token.as_deref())
                })
                .await
            };
            this.update(cx, |this, cx| {
                let result = result.map_err(|e| format!("{e:#}").into());
                this.updates.insert(name, result);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Downloads the selected skill's update, then asks to install it.
    pub(super) fn download_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Selection::Skill(name)) = self.selected.clone() else {
            return;
        };
        let (Some(env), Some(Ok(Some(update)))) = (self.env.clone(), self.updates.get(&name))
        else {
            return;
        };
        if self.busy || self.fetching {
            return;
        }
        let update = update.clone();
        self.fetching = true;
        self.fetch_error = None;
        let selection = Selection::Skill(name.clone());
        self.fetch_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let token = GitHubAccess::load().token(&env).ok().flatten();
                    fetch_update(&env, &update, token.as_deref())
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.selected.as_ref() != Some(&selection) {
                    if let Ok(download) = result {
                        download.discard();
                    }
                    return;
                }
                this.fetching = false;
                match result {
                    Ok(download) => {
                        this.set_fetched(Some(download.clone()));
                        open_change_dialog(
                            this,
                            Change::UpdateSkill { name, download },
                            window,
                            cx,
                        );
                    }
                    Err(e) => this.fetch_error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Whether a newer version is on GitHub, with the button that installs it.
    fn update_status(&self, skill: &Skill, cx: &mut Context<Self>) -> Option<AnyElement> {
        skill.source.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let line = |text: SharedString| h_flex().gap_2().text_sm().text_color(muted).child(text);
        let element = if self.fetching {
            line("Downloading the update…".into()).child(Spinner::new().small())
        } else if let Some(error) = &self.fetch_error {
            line(error.clone()).text_color(cx.theme().danger)
        } else {
            match self.updates.get(&skill.name) {
                None if self.update_task.is_some() => {
                    line("Checking for updates…".into()).child(Spinner::new().small())
                }
                None => return None,
                Some(Ok(None)) => line("Up to date with GitHub".into()),
                Some(Err(error)) => line(format!("Could not check for updates: {error}").into()),
                Some(Ok(Some(_))) => line("A newer version is on GitHub".into())
                    .text_color(cx.theme().foreground)
                    .child(div().flex_1())
                    .child(
                        Button::new("update")
                            .primary()
                            .small()
                            .icon(Lucide::Download)
                            .label("Update…")
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.download_update(window, cx)),
                            ),
                    ),
            }
        };
        Some(element.into_any_element())
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

    pub(super) fn confirm_uninstall(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let change = match &self.selected {
            Some(Selection::Skill(name)) => self
                .skill(name)
                .filter(|s| !s.default)
                .cloned()
                .map(Change::UninstallSkill),
            Some(Selection::Plugin(id)) => self
                .plugin(id)
                .filter(|p| !p.is_default())
                .cloned()
                .map(Change::UninstallPlugin),
            _ => None,
        };
        if let Some(change) = change {
            open_change_dialog(self, change, window, cx);
        }
    }

    pub(super) fn render_skills(&self, cx: &mut Context<Self>) -> AnyElement {
        let skills: Vec<_> = self.visible_skills(cx).collect();
        let first_default = skills.iter().position(|s| s.default);
        let mut rows: Vec<_> = skills.into_iter().map(|s| self.skill_row(s, cx)).collect();
        with_default_heading(&mut rows, first_default, cx);
        v_flex()
            .size_full()
            .child(self.page_header("Skills", Some(rows.len()), cx))
            .child(self.column_header(&self.installed_columns(), cx))
            .child(list_body("skill-list", rows, &self.list_scroll, cx))
            .into_any_element()
    }

    fn skill_row(&self, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let name = skill.name.clone();
        // A shared skill can be turned on for Claude by linking it in.
        let togglable: Vec<Provider> = self
            .agents
            .iter()
            .copied()
            .filter(|&p| {
                !skill.default
                    && (skill.enabled(p).is_some()
                        || (p == Provider::Claude && skill.shared.is_some()))
            })
            .collect();
        let all_on = togglable.iter().all(|&p| skill.enabled(p) == Some(true));
        let mut switches = Vec::new();
        if self.agents.len() > 1 {
            switches.push(self.switch(
                format!("skill-all-{name}"),
                all_on,
                togglable.is_empty(),
                "All agents",
                {
                    let name = name.clone();
                    let togglable = togglable.clone();
                    cx.listener(move |this, on: &bool, window, cx| {
                        this.set_skill_enabled(&name, &togglable, *on, window, cx)
                    })
                },
            ));
        }
        for &provider in &self.agents {
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
        let selection = Selection::Skill(name.clone());
        switches.push(self.row_uninstall(
            format!("uninstall-skill-{name}"),
            &selection,
            skill.default,
            cx,
        ));
        let selected = self.selected.as_ref() == Some(&selection);
        self.row(
            format!("skill-{name}"),
            selected,
            if skill.default {
                skill.title.clone()
            } else {
                skill.name.clone()
            },
            if skill.default {
                None
            } else if skill.shared.is_none() {
                Some("unmanaged")
            } else {
                self.has_update(&skill.name).then_some("update available")
            },
            skill.description.clone(),
            switches,
            cx.listener(move |this, _, _, cx| this.select(Some(selection.clone()), cx)),
            cx,
        )
    }

    pub(super) fn render_plugins(&self, cx: &mut Context<Self>) -> AnyElement {
        let plugins: Vec<_> = self.visible_plugins(cx).collect();
        let first_default = plugins.iter().position(|p| p.is_default());
        let mut rows: Vec<_> = plugins
            .into_iter()
            .map(|p| self.plugin_row(p, cx))
            .collect();
        with_default_heading(&mut rows, first_default, cx);
        let warnings: Vec<_> = self
            .plugin_errors
            .iter()
            .map(|error| warning(error.clone(), cx))
            .collect();
        v_flex()
            .size_full()
            .child(self.page_header("Plugins", Some(rows.len()), cx))
            .children(warnings)
            .child(self.column_header(&self.installed_columns(), cx))
            .child(list_body("plugin-list", rows, &self.list_scroll, cx))
            .into_any_element()
    }

    fn plugin_row(&self, plugin: &Plugin, cx: &mut Context<Self>) -> AnyElement {
        let id = plugin.id.clone();
        let installed: Vec<Provider> = plugin
            .installs
            .iter()
            .filter(|(_, i)| !i.default)
            .map(|(&p, _)| p)
            .collect();
        let all_on = plugin.installs.values().all(|i| i.enabled);
        let mut switches = Vec::new();
        if self.agents.len() > 1 {
            let locked = installed.is_empty();
            switches.push(
                self.switch(format!("plugin-all-{id}"), all_on, locked, "All agents", {
                    let id = id.clone();
                    let installed = installed.clone();
                    cx.listener(move |this, on: &bool, window, cx| {
                        this.set_plugin_enabled(&id, &installed, *on, window, cx)
                    })
                }),
            );
        }
        for &provider in &self.agents {
            let id = id.clone();
            let install = plugin.installs.get(&provider);
            switches.push(self.switch(
                format!("plugin-{}-{id}", provider.cli()),
                install.is_some_and(|i| i.enabled),
                install.is_none_or(|i| i.default),
                provider.label(),
                cx.listener(move |this, on: &bool, window, cx| {
                    this.set_plugin_enabled(&id, &[provider], *on, window, cx)
                }),
            ));
        }
        let selection = Selection::Plugin(id.clone());
        switches.push(self.row_uninstall(
            format!("uninstall-plugin-{id}"),
            &selection,
            plugin.is_default(),
            cx,
        ));
        let selected = self.selected.as_ref() == Some(&selection);
        self.row(
            format!("plugin-{id}"),
            selected,
            plugin.name.clone(),
            Some(plugin.marketplace.as_str()).filter(|m| !m.is_empty()),
            plugin.description.clone(),
            switches,
            cx.listener(move |this, _, _, cx| this.select(Some(selection.clone()), cx)),
            cx,
        )
    }

    /// The agent columns, then an untitled one for each row's uninstall button.
    fn installed_columns(&self) -> Vec<&'static str> {
        let mut columns = self.agent_columns();
        columns.push("");
        columns
    }

    /// A row's own uninstall button: selects the row and asks to confirm. Defaults get an empty
    /// cell, so the columns stay aligned.
    fn row_uninstall(
        &self,
        id: String,
        selection: &Selection,
        default: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if default {
            return cell().into_any_element();
        }
        let selection = selection.clone();
        cell()
            .child(
                Button::new(SharedString::from(id))
                    .ghost()
                    .small()
                    .icon(Lucide::Trash)
                    .tooltip("Uninstall… (Delete)")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(Some(selection.clone()), cx);
                        this.confirm_uninstall(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn uninstall_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("uninstall")
            .danger()
            .icon(Lucide::Trash)
            .label("Uninstall…")
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, window, cx| this.confirm_uninstall(window, cx)))
    }

    pub(super) fn skill_detail(&self, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let Some(env) = &self.env else {
            return div().into_any_element();
        };
        let mut locations: Vec<_> = skill
            .installs
            .iter()
            .filter(|install| self.agents.contains(&install.provider))
            .map(|install| {
                let kind = match std::fs::read_link(&install.path) {
                    Ok(target) => format!("link to {}", env.tilde(&target)),
                    Err(_) if Some(&install.path) == skill.shared.as_ref() => "shared copy".into(),
                    Err(_) if install.shared => "copy".into(),
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
        if locations.is_empty()
            && let Some(shared) = &skill.shared
        {
            locations.push(fact("Shared", env.tilde(shared), cx));
        }
        let subtitle = if skill.default {
            Some("Ships with Codex; cannot be removed or turned off".into())
        } else {
            skill.source.clone().or(skill
                .shared
                .is_none()
                .then(|| "Not managed by skillshelf".into()))
        };
        let uninstall = (!skill.default).then(|| self.uninstall_button(cx));
        v_flex()
            .size_full()
            .child(self.detail_header(skill.title.clone(), subtitle, uninstall, cx))
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
                            .children(self.update_status(skill, cx))
                            .child(v_flex().gap_1().children(locations))
                            .child(divider(cx))
                            .child(
                                TextView::markdown("skill-md", self.skill_md.clone())
                                    .selectable(true),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn plugin_detail(&self, plugin: &Plugin, cx: &mut Context<Self>) -> AnyElement {
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
        let uninstall = (!plugin.is_default()).then(|| self.uninstall_button(cx));
        v_flex()
            .size_full()
            .child(self.detail_header(
                plugin.name.clone(),
                Some(plugin.marketplace.clone()),
                uninstall,
                cx,
            ))
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .text_color(cx.theme().foreground)
                    .child(plugin.description.clone())
                    .child(v_flex().gap_1().children(installs)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use skillshelf_core::{Provider, Skill, SkillInstall};

    use super::for_agents;

    fn install(provider: Provider, path: &str, shared: bool) -> SkillInstall {
        SkillInstall {
            provider,
            path: PathBuf::from(path),
            shared,
            enabled: true,
        }
    }

    fn skill(name: &str, shared: bool, installs: Vec<SkillInstall>) -> Skill {
        Skill {
            name: name.into(),
            title: name.into(),
            description: String::new(),
            shared: shared.then(|| PathBuf::from(format!("agents/{name}"))),
            source: None,
            installs,
            default: false,
        }
    }

    fn skills() -> Vec<Skill> {
        vec![
            // In the shared folder (Codex reads it) and linked into Claude.
            skill(
                "linked",
                true,
                vec![
                    install(Provider::Codex, "agents/linked", true),
                    install(Provider::Claude, "claude/linked", true),
                ],
            ),
            // Only in the shared folder.
            skill(
                "stored",
                true,
                vec![install(Provider::Codex, "agents/stored", true)],
            ),
            // Claude's own folder, unrelated to the shared one.
            skill(
                "own",
                false,
                vec![install(Provider::Claude, "claude/own", false)],
            ),
        ]
    }

    fn names(skills: &[Skill]) -> Vec<&str> {
        skills.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn lists_what_the_managed_agents_have_or_can_link() {
        assert_eq!(
            names(&for_agents(skills(), &Provider::ALL)),
            ["linked", "stored", "own"]
        );
        assert_eq!(
            names(&for_agents(skills(), &[Provider::Claude])),
            ["linked", "stored", "own"]
        );
        assert_eq!(
            names(&for_agents(skills(), &[Provider::Codex])),
            ["linked", "stored"]
        );
        assert!(for_agents(skills(), &[]).is_empty());
    }

    #[test]
    fn keeps_other_agents_links_to_the_shared_folder_only() {
        let listed = for_agents(skills(), &[Provider::Claude]);
        // The shared folder stays, so uninstalling from Claude knows to remove it.
        assert_eq!(listed[0].installs.len(), 2);
        let mut own = skills();
        own[2]
            .installs
            .push(install(Provider::Codex, "codex/own", false));
        let listed = for_agents(own, &[Provider::Claude]);
        assert_eq!(
            listed[2].installs,
            [install(Provider::Claude, "claude/own", false)]
        );
    }
}
