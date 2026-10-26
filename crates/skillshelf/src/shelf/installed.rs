//! Installed skills and plugins: lists with per-provider switches, details, uninstall.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable, button::Button, button::ButtonVariants, scroll::ScrollableElement,
    text::TextView, v_flex,
};
use gpui_kit::*;
use skillshelf_core::{Plugin, Provider, Skill, toggle_plugin, toggle_skill};

use super::change::{Change, open_change_dialog};
use super::{Selection, Shelf, divider, fact, list_body, matches, warning};

impl Shelf {
    pub(super) fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub(super) fn plugin(&self, id: &str) -> Option<&Plugin> {
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
        let change = match &self.selected {
            Some(Selection::Skill(name)) => self.skill(name).cloned().map(Change::UninstallSkill),
            Some(Selection::Plugin(id)) => self.plugin(id).cloned().map(Change::UninstallPlugin),
            _ => None,
        };
        if let Some(change) = change {
            open_change_dialog(self, change, window, cx);
        }
    }

    pub(super) fn render_skills(&self, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.filter_text(cx);
        let rows: Vec<_> = self
            .skills
            .iter()
            .filter(|s| matches(&filter, &[&s.name, &s.description]))
            .map(|s| self.skill_row(s, cx))
            .collect();
        v_flex()
            .size_full()
            .child(self.page_header("Skills", Some(rows.len()), cx))
            .child(self.column_header(&["All", "Claude", "Codex"], cx))
            .child(list_body("skill-list", rows, cx))
            .into_any_element()
    }

    fn skill_row(&self, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let name = skill.name.clone();
        // A shared skill can be turned on for Claude by linking it in.
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
        let selection = Selection::Skill(name.clone());
        let selected = self.selected.as_ref() == Some(&selection);
        self.row(
            format!("skill-{name}"),
            selected,
            skill.name.clone(),
            skill.shared.is_none().then_some("unmanaged"),
            skill.description.clone(),
            switches,
            cx.listener(move |this, _, _, cx| this.select(Some(selection.clone()), cx)),
            cx,
        )
    }

    pub(super) fn render_plugins(&self, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.filter_text(cx);
        let rows: Vec<_> = self
            .plugins
            .iter()
            .filter(|p| matches(&filter, &[&p.id, &p.description]))
            .map(|p| self.plugin_row(p, cx))
            .collect();
        let warnings: Vec<_> = self
            .plugin_errors
            .iter()
            .map(|error| warning(error.clone(), cx))
            .collect();
        v_flex()
            .size_full()
            .child(self.page_header("Plugins", Some(rows.len()), cx))
            .children(warnings)
            .child(self.column_header(&["All", "Claude", "Codex"], cx))
            .child(list_body("plugin-list", rows, cx))
            .into_any_element()
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
        let selection = Selection::Plugin(id.clone());
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
        let subtitle = skill.source.clone().or(skill
            .shared
            .is_none()
            .then(|| "Not managed by skillshelf".into()));
        let uninstall = self.uninstall_button(cx);
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
        let uninstall = self.uninstall_button(cx);
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
