//! Profiles: the switcher in the title bar, and the page that adds, edits and removes them.

use std::collections::BTreeMap;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use xolkit_core::{Plugin, Profile, Provider, Skill, States, skill_state};

use super::installed::{for_agents, plugins_for};
use super::{Page, Selection, Shelf, cell, divider, list_body, matches, warning};
use crate::profiles::Profiles;

impl Shelf {
    /// Narrows what was read to the active profile's agents.
    pub(super) fn sync_lists(&mut self) {
        let agents = self.agents().to_vec();
        self.skills = for_agents(self.scanned.clone(), &agents);
        self.plugins = plugins_for(&self.all_plugins, &agents);
    }

    /// Keeps the active profile in step with how skills and plugins are set now, including
    /// changes made outside xolkit.
    pub(super) fn record_profile(&mut self) {
        if self
            .profiles
            .active_mut()
            .record(&self.scanned, &self.all_plugins)
            && let Err(e) = self.profiles.save()
        {
            eprintln!("xolkit: could not save the profile: {e:#}");
        }
    }

    /// Changes a profile and saves it; on failure, says why and keeps the profile as it was.
    pub(super) fn edit_profile(
        &mut self,
        ix: usize,
        edit: impl FnOnce(&mut Profile),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut profiles = self.profiles.clone();
        edit(&mut profiles.list[ix]);
        self.save_profiles(profiles, window, cx)
    }

    fn save_profiles(
        &mut self,
        profiles: Profiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Err(e) = profiles.save() {
            window.push_notification(Notification::error(format!("{e:#}")), cx);
            return false;
        }
        self.profiles = profiles;
        cx.notify();
        true
    }

    pub(super) fn visible_profiles(&self, cx: &App) -> impl Iterator<Item = (usize, &Profile)> {
        let filter = self.filter_text(cx);
        self.profiles
            .list
            .iter()
            .enumerate()
            .filter(move |(_, p)| matches(&filter, &[&p.name]))
    }

    /// Applies another profile: shows the steps that bring skills and plugins in line with it,
    /// then runs them.
    pub(super) fn switch_profile(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix == self.profiles.active || !self.settled() {
            return;
        }
        let Some(env) = self.env.clone() else {
            return;
        };
        let profile = &self.profiles.list[ix];
        let steps = profile.apply(&env, &self.scanned, &self.all_plugins);
        let name = profile.name.clone();
        let done: SharedString = format!("Switched to {name}").into();
        if steps.is_empty() {
            if self.activate(ix, window, cx) {
                window.push_notification(Notification::success(done), cx);
                self.reload(cx);
            }
            return;
        }
        // The dialog doesn't scroll, so a long list ends in a count.
        let mut lines: Vec<String> = steps
            .iter()
            .take(MAX_STEPS)
            .map(|step| step.describe(&env))
            .collect();
        if steps.len() > MAX_STEPS {
            lines.push(format!("…and {} more", steps.len() - MAX_STEPS));
        }
        let title = format!("Switch to “{name}”?");
        let shelf = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let theme = cx.theme();
            let (shelf, steps, done) = (shelf.clone(), steps.clone(), done.clone());
            alert
                .title(title.clone())
                .description(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .children(lines.clone()),
                )
                .width(px(640.))
                .show_cancel(true)
                .ok_text("Switch")
                .on_ok(move |_, window, cx| {
                    shelf.update(cx, |shelf, cx| {
                        shelf.switching_to = Some(ix);
                        shelf.run(steps.clone(), Some(done.clone()), window, cx);
                    });
                    true
                })
        });
    }

    /// Whether the lists match how things are now: read, and no steps or reload under way. Switching
    /// and copying profiles work from these lists.
    fn settled(&self) -> bool {
        self.env.is_some() && !self.busy && !self.loading
    }

    /// Makes a profile the active one, without changing any skill or plugin.
    pub(super) fn activate(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let agents_before = self.agents().to_vec();
        let mut profiles = self.profiles.clone();
        profiles.active = ix;
        if !self.save_profiles(profiles, window, cx) {
            return false;
        }
        if self.agents() != agents_before {
            self.offers = None;
            self.offer_errors.clear();
        }
        self.sync_lists();
        true
    }

    /// Asks for a name, then adds a profile (`renaming` is `None`) or renames one.
    fn open_name_dialog(
        &mut self,
        renaming: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = renaming.map(|ix| self.profiles.list[ix].name.clone());
        let (title, ok, note) = match &current {
            Some(name) => (format!("Rename “{name}”"), "Rename", None),
            None => (
                "New profile".to_owned(),
                "Add",
                Some(format!(
                    "It starts as a copy of “{}”.",
                    self.profiles.active().name
                )),
            ),
        };
        let input = self.profile_name.clone();
        input.update(cx, |input, cx| {
            input.set_value(current.unwrap_or_default(), window, cx)
        });
        let shelf = cx.entity();
        window.open_alert_dialog(cx, {
            let input = input.clone();
            move |alert, _, cx| {
                let (input, shelf) = (input.clone(), shelf.clone());
                alert
                    .title(title.clone())
                    .description(v_flex().gap_2().child(Input::new(&input)).when_some(
                        note.clone(),
                        |this, note| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(note),
                            )
                        },
                    ))
                    .show_cancel(true)
                    .ok_text(ok)
                    .on_ok(move |_, window, cx| {
                        let name = input.read(cx).value().to_string();
                        shelf.update(cx, |shelf, cx| {
                            shelf.name_profile(renaming, &name, window, cx)
                        })
                    })
            }
        });
        // The dialog takes focus as it opens; hand it to the field after that.
        window.defer(cx, move |window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx))
        });
    }

    /// Adds or renames a profile and shows it; says what is wrong with the name otherwise.
    fn name_profile(
        &mut self,
        renaming: Option<usize>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut profiles = self.profiles.clone();
        let named = match renaming {
            Some(ix) => profiles.rename(ix, name).map(|()| ix),
            None => profiles.add(name),
        };
        let ix = match named {
            Ok(ix) => ix,
            Err(e) => {
                window.push_notification(Notification::error(format!("{e:#}")), cx);
                return false;
            }
        };
        let name = profiles.list[ix].name.clone();
        if !self.save_profiles(profiles, window, cx) {
            return false;
        }
        self.open_page(Page::Profiles, window, cx);
        self.select(Some(Selection::Profile(name)), cx);
        true
    }

    fn confirm_remove_profile(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.profiles.list[ix].name.clone();
        let title = format!("Remove “{name}”?");
        let shelf = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (shelf, name) = (shelf.clone(), name.clone());
            alert
                .title(title.clone())
                .description("Skills and plugins stay as they are; only the profile goes.")
                .show_cancel(true)
                .ok_text("Remove")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    shelf.update(cx, |shelf, cx| shelf.remove_profile(&name, window, cx));
                    true
                })
        });
    }

    fn remove_profile(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.profiles.position(name) else {
            return;
        };
        let mut profiles = self.profiles.clone();
        if let Err(e) = profiles.remove(ix) {
            window.push_notification(Notification::error(format!("{e:#}")), cx);
            return;
        }
        if self.save_profiles(profiles, window, cx) {
            self.select(None, cx);
            window.push_notification(Notification::success(format!("Removed {name}")), cx);
        }
    }

    /// The title bar's switcher: the profiles, the active one checked.
    pub(super) fn profile_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<String> = self.profiles.list.iter().map(|p| p.name.clone()).collect();
        let active = self.profiles.active;
        let shelf = cx.weak_entity();
        Button::new("profile")
            .ghost()
            .small()
            .icon(Icon::new(Lucide::Layers))
            .label(format!("Profile: {}", self.profiles.active().name))
            .tooltip("Profile")
            .disabled(!self.settled())
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu.scrollable(true).max_h(px(420.));
                for (ix, name) in names.iter().enumerate() {
                    let shelf = shelf.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(ix == active)
                            .on_click(move |_, window, cx| {
                                shelf
                                    .update(cx, |shelf, cx| shelf.switch_profile(ix, window, cx))
                                    .ok();
                            }),
                    );
                }
                let shelf = shelf.clone();
                menu.separator()
                    .item(
                        PopupMenuItem::new("Manage profiles…").on_click(move |_, window, cx| {
                            shelf
                                .update(cx, |shelf, cx| shelf.open_page(Page::Profiles, window, cx))
                                .ok();
                        }),
                    )
            })
    }

    pub(super) fn render_profiles(&self, cx: &mut Context<Self>) -> AnyElement {
        let profiles: Vec<_> = self.visible_profiles(cx).collect();
        let rows: Vec<_> = profiles
            .into_iter()
            .map(|(ix, profile)| self.profile_row(ix, profile, cx))
            .collect();
        v_flex()
            .size_full()
            .child(
                self.page_header("Profiles", Some(rows.len()), cx).child(
                    Button::new("new-profile")
                        .small()
                        .icon(IconName::Plus)
                        .label("New profile…")
                        .disabled(!self.settled())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_name_dialog(None, window, cx)
                        })),
                ),
            )
            .child(list_body("profile-list", rows, &self.list_scroll, cx))
            .into_any_element()
    }

    fn profile_row(&self, ix: usize, profile: &Profile, cx: &mut Context<Self>) -> AnyElement {
        let active = ix == self.profiles.active;
        let tag = match (ix == 0, active) {
            (true, true) => Some("default, in use"),
            (true, false) => Some("default"),
            (false, true) => Some("in use"),
            (false, false) => None,
        };
        let control = if active {
            cell()
                .text_color(cx.theme().muted_foreground)
                .child(IconName::Check)
                .into_any_element()
        } else {
            cell()
                .child(
                    Button::new(SharedString::from(format!("use-profile-{ix}")))
                        .small()
                        .label("Switch")
                        .disabled(!self.settled())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch_profile(ix, window, cx)
                        })),
                )
                .into_any_element()
        };
        let selection = Selection::Profile(profile.name.clone());
        let selected = self.selected.as_ref() == Some(&selection);
        self.row(
            format!("profile-{ix}"),
            selected,
            profile.name.clone(),
            tag,
            summary(profile),
            vec![control],
            cx.listener(move |this, _, _, cx| this.select(Some(selection.clone()), cx)),
            cx,
        )
    }

    pub(super) fn profile_detail(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let profile = &self.profiles.list[ix];
        let active = ix == self.profiles.active;
        let muted = cx.theme().muted_foreground;
        let switch = Button::new("switch-profile")
            .primary()
            .label(if active { "In use" } else { "Switch" })
            .disabled(active || !self.settled())
            .on_click(cx.listener(move |this, _, window, cx| this.switch_profile(ix, window, cx)));
        let note = if ix == 0 {
            "The default profile can't be renamed or removed."
        } else if active {
            "Switch to another profile to remove this one."
        } else {
            ""
        };
        let manage = h_flex()
            .gap_2()
            .child(
                Button::new("rename-profile")
                    .small()
                    .icon(Icon::new(Lucide::Pencil))
                    .label("Rename…")
                    .disabled(ix == 0 || self.busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_name_dialog(Some(ix), window, cx)
                    })),
            )
            .child(
                Button::new("remove-profile")
                    .small()
                    .danger()
                    .icon(Lucide::Trash)
                    .label("Remove…")
                    .disabled(ix == 0 || active || self.busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_remove_profile(ix, window, cx)
                    })),
            );
        let contents = if active {
            vec![
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(
                        "This profile is in use: what you turn on and off on the Skills and \
                         Plugins pages is saved to it.",
                    )
                    .into_any_element(),
            ]
        } else {
            self.profile_contents(ix, cx)
        };
        v_flex()
            .size_full()
            .child(self.detail_header(
                profile.name.clone(),
                (ix == 0).then(|| "Default profile".to_owned()),
                switch,
                cx,
            ))
            .child(
                div()
                    .id("profile-detail")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .p_4()
                            .gap_3()
                            .child(manage)
                            .when(!note.is_empty(), |this| {
                                this.child(div().text_xs().text_color(muted).child(note))
                            })
                            .child(divider(cx))
                            .child(div().font_semibold().child("Agents"))
                            .child(v_flex().gap_3().children(self.agent_rows(ix, cx)))
                            .child(divider(cx))
                            .children(contents),
                    ),
            )
            .into_any_element()
    }

    /// The skills and plugins of a profile not in use, with a switch per agent that changes the
    /// profile only.
    fn profile_contents(&self, ix: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let profile = &self.profiles.list[ix];
        let muted = cx.theme().muted_foreground;
        let note = |text: &'static str| {
            div()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        if profile.agents.is_empty() {
            return vec![note("Turn on an agent to pick its skills and plugins.")];
        }
        // Default installs can't be turned off, so a profile has no say over them.
        let mut skills = for_agents(self.scanned.clone(), &profile.agents);
        skills.retain(|s| !s.default);
        let mut plugins = plugins_for(&self.all_plugins, &profile.agents);
        plugins.retain(|p| p.installs.values().any(|i| !i.default));
        let mut contents = vec![note(
            "Switching to this profile turns these on and off. Skills and plugins installed \
             later stay as they are.",
        )];
        contents.extend(
            self.plugin_errors
                .iter()
                .filter(|(provider, _)| profile.agents.contains(provider))
                .map(|(_, error)| warning(error.clone(), cx).into_any_element()),
        );
        contents.push(self.item_header("Skills", &profile.agents, cx));
        contents.extend(skills.iter().map(|skill| self.profile_skill(ix, skill, cx)));
        contents.push(self.item_header("Plugins", &profile.agents, cx));
        contents.extend(
            plugins
                .iter()
                .map(|plugin| self.profile_plugin(ix, plugin, cx)),
        );
        contents
    }

    fn item_header(
        &self,
        title: &'static str,
        agents: &[Provider],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .pt_2()
            .gap_3()
            .child(div().flex_1().font_semibold().child(title))
            .children(agents.iter().map(|agent| {
                cell()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(agent.label())
            }))
            .into_any_element()
    }

    fn profile_skill(&self, ix: usize, skill: &Skill, cx: &mut Context<Self>) -> AnyElement {
        let profile = &self.profiles.list[ix];
        let switches = profile.agents.iter().map(|&agent| {
            let name = skill.name.clone();
            self.switch(
                format!("profile-skill-{}-{name}", agent.cli()),
                profile.skill_on(skill, agent) == Some(true),
                skill_state(skill, agent).is_none(),
                agent.label(),
                cx.listener(move |this, on: &bool, window, cx| {
                    this.edit_profile(ix, |p| p.set_skill(&name, agent, *on), window, cx);
                }),
            )
        });
        item_row(skill.name.clone(), switches.collect())
    }

    fn profile_plugin(&self, ix: usize, plugin: &Plugin, cx: &mut Context<Self>) -> AnyElement {
        let profile = &self.profiles.list[ix];
        let switches = profile.agents.iter().map(|&agent| {
            let id = plugin.id.clone();
            let on = profile.plugin_on(plugin, agent);
            self.switch(
                format!("profile-plugin-{}-{id}", agent.cli()),
                on == Some(true),
                on.is_none(),
                agent.label(),
                cx.listener(move |this, on: &bool, window, cx| {
                    this.edit_profile(ix, |p| p.set_plugin(&id, agent, *on), window, cx);
                }),
            )
        });
        item_row(plugin.name.clone(), switches.collect())
    }
}

/// Most steps the switch dialog lists.
const MAX_STEPS: usize = 14;

fn item_row(name: String, switches: Vec<AnyElement>) -> AnyElement {
    h_flex()
        .gap_3()
        .text_sm()
        .child(div().flex_1().min_w_0().truncate().child(name))
        .children(switches)
        .into_any_element()
}

/// "Claude, Codex · 3 skills and 1 plugin on", from what the profile has saved.
fn summary(profile: &Profile) -> String {
    if profile.agents.is_empty() {
        return "No agents".to_owned();
    }
    let on = |items: &BTreeMap<String, States>| {
        items
            .values()
            .filter(|states| {
                profile
                    .agents
                    .iter()
                    .any(|agent| states.get(agent) == Some(&true))
            })
            .count()
    };
    let agents: Vec<&str> = profile.agents.iter().map(|p| p.label()).collect();
    format!(
        "{} · {} and {} on",
        agents.join(", "),
        count(on(&profile.skills), "skill"),
        count(on(&profile.plugins), "plugin")
    )
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use xolkit_core::{Profile, Provider};

    use super::summary;

    #[test]
    fn summarizes_agents_and_what_is_on() {
        let mut profile = Profile::new("Work", vec![]);
        assert_eq!(summary(&profile), "No agents");
        profile.set_agent(Provider::Claude, true);
        profile.set_skill("a", Provider::Claude, true);
        profile.set_skill("b", Provider::Claude, false);
        // Saved for an agent the profile no longer manages.
        profile.set_skill("c", Provider::Codex, true);
        profile.set_plugin("p@m", Provider::Claude, true);
        assert_eq!(summary(&profile), "Claude · 1 skill and 1 plugin on");
    }
}
