//! The install, update and uninstall dialogs: which providers it applies to, and every step that
//! will run.

use std::collections::BTreeSet;

use anyhow::Result;
use gpui_kit::component::{
    ActiveTheme, IconName, WindowExt, button::ButtonVariant, checkbox::Checkbox, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use xolkit_core::{
    Action as Step, AvailablePlugin, Env, FetchedSkill, Plugin, Provider, Skill, install_plugin,
    install_skill, uninstall_plugin, uninstall_skill, update_skill,
};

use super::discover::is_code;
use super::{Shelf, icon_line};

pub(super) enum Change {
    UninstallSkill(Skill),
    UninstallPlugin(Plugin),
    InstallSkill(FetchedSkill),
    InstallPlugin(AvailablePlugin),
    /// Replaces installed skill `name` with a downloaded newer version.
    UpdateSkill {
        name: String,
        download: FetchedSkill,
    },
}

impl Change {
    fn name(&self) -> &str {
        match self {
            Change::UninstallSkill(skill) => &skill.name,
            Change::UninstallPlugin(plugin) => &plugin.id,
            Change::InstallSkill(skill) => &skill.name,
            Change::InstallPlugin(plugin) => &plugin.id,
            Change::UpdateSkill { name, .. } => name,
        }
    }

    fn installs(&self) -> bool {
        !matches!(self, Change::UninstallSkill(_) | Change::UninstallPlugin(_))
    }

    /// Agents the user can pick from, out of the ones xolkit manages.
    fn choices(&self, agents: &[Provider]) -> BTreeSet<Provider> {
        let offered: BTreeSet<Provider> = match self {
            Change::UninstallSkill(skill) => skill.installs.iter().map(|i| i.provider).collect(),
            Change::UninstallPlugin(plugin) => plugin.installs.keys().copied().collect(),
            Change::InstallSkill(_) => Provider::ALL.into(),
            Change::InstallPlugin(plugin) => plugin
                .providers
                .iter()
                .filter(|(_, installed)| !**installed)
                .map(|(p, _)| *p)
                .collect(),
            Change::UpdateSkill { .. } => BTreeSet::new(),
        };
        offered.into_iter().filter(|p| agents.contains(p)).collect()
    }
}

pub(super) struct ChangeForm {
    env: Env,
    change: Change,
    /// The agents xolkit manages.
    agents: Vec<Provider>,
    providers: BTreeSet<Provider>,
    delete_data: bool,
}

impl ChangeForm {
    fn steps(&self) -> Result<Vec<Step>> {
        let providers: Vec<Provider> = self.providers.iter().copied().collect();
        Ok(match &self.change {
            Change::UninstallSkill(skill) => {
                // The skill only holds an unmanaged agent's links to its shared folder, which go
                // when the folder does.
                let unmanaged = Provider::ALL
                    .into_iter()
                    .filter(|p| !self.agents.contains(p));
                let providers: Vec<Provider> = providers.iter().copied().chain(unmanaged).collect();
                uninstall_skill(skill, &providers)
            }
            Change::UninstallPlugin(plugin) => {
                uninstall_plugin(plugin, &providers, self.delete_data)
            }
            Change::InstallSkill(skill) => {
                install_skill(&self.env, skill, &providers, &self.agents)?
            }
            Change::InstallPlugin(plugin) => install_plugin(plugin, &providers),
            Change::UpdateSkill { name, download } => update_skill(&self.env, name, download)?,
        })
    }
}

impl Render for ChangeForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let claude_plugin = matches!(&self.change,
            Change::UninstallPlugin(p) if p.installs.contains_key(&Provider::Claude));
        let steps = self.steps();
        let theme = cx.theme();
        // An update brings files nobody has reviewed yet; list them, flagging code.
        let new_files = match &self.change {
            Change::UpdateSkill { download, .. } => download.files.as_slice(),
            _ => &[],
        };
        let summary = match &steps {
            Ok(steps) if steps.is_empty() => v_flex().child("Pick at least one agent."),
            Ok(steps) => v_flex().children(steps.iter().map(|step| step.describe(&self.env))),
            Err(e) => v_flex().text_color(theme.danger).child(format!("{e:#}")),
        };
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_4()
                    .children(
                        self.change
                            .choices(&self.agents)
                            .into_iter()
                            .map(|provider| {
                                Checkbox::new(SharedString::from(format!(
                                    "change-{}",
                                    provider.cli()
                                )))
                                .label(provider.label())
                                .checked(self.providers.contains(&provider))
                                .on_change(cx.listener(
                                    move |this, on: &bool, _, cx| {
                                        if *on {
                                            this.providers.insert(provider);
                                        } else {
                                            this.providers.remove(&provider);
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
            )
            .when(claude_plugin, |this| {
                this.child(
                    Checkbox::new("change-data")
                        .label("Also delete the plugin's saved data (Claude)")
                        .checked(self.delete_data)
                        .on_change(cx.listener(|this, on: &bool, _, cx| {
                            this.delete_data = *on;
                            cx.notify();
                        })),
                )
            })
            .child(
                summary
                    .gap_1()
                    .p_2()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .text_sm()
                    .text_color(theme.muted_foreground),
            )
            .when(!new_files.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Files in the new version (scripts an agent may run are flagged):")
                        .children(new_files.iter().map(|file| {
                            h_flex()
                                .gap_2()
                                .line_height(icon_line())
                                .when(is_code(file), |row| {
                                    row.text_color(theme.warning).child(IconName::TriangleAlert)
                                })
                                .child(file.clone())
                        })),
                )
            })
    }
}

/// Asks for confirmation, then runs the change.
pub(super) fn open_change_dialog(
    shelf: &mut Shelf,
    change: Change,
    window: &mut Window,
    cx: &mut Context<Shelf>,
) {
    let Some(env) = shelf.env.clone() else {
        return;
    };
    let name = change.name().to_owned();
    let installs = change.installs();
    let moves_download = matches!(change, Change::InstallSkill(_) | Change::UpdateSkill { .. });
    let updates = matches!(change, Change::UpdateSkill { .. });
    let (verb, done) = if updates {
        ("Update", format!("Updated {name}"))
    } else if installs {
        ("Install", format!("Installed {name}"))
    } else {
        ("Uninstall", format!("Uninstalled {name}"))
    };
    let title = format!("{verb} “{name}”?");
    let providers = change.choices(shelf.agents());
    let agents = shelf.agents().to_vec();
    let form = cx.new(|_| ChangeForm {
        env,
        change,
        agents,
        providers,
        delete_data: true,
    });
    let shelf = cx.entity();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (form, shelf, done, name) = (form.clone(), shelf.clone(), done.clone(), name.clone());
        alert
            .title(title.clone())
            .description(form.clone())
            .width(px(640.))
            .show_cancel(true)
            .ok_text(verb)
            .ok_variant(if installs {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Danger
            })
            .on_ok(move |_, window, cx| {
                let Ok(steps) = form.read(cx).steps() else {
                    return false;
                };
                if steps.is_empty() {
                    return false;
                }
                shelf.update(cx, |shelf, cx| {
                    if moves_download {
                        // The change moves the download into place; keep it from being discarded.
                        shelf.fetched = None;
                    }
                    if updates {
                        // Checked again once the reload after the update finishes.
                        shelf.updates.remove(&name);
                    }
                    shelf.run(steps, Some(done.clone().into()), window, cx);
                });
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use xolkit_core::{Action, Env, Provider, Skill, SkillInstall};

    use super::{Change, ChangeForm};

    fn uninstall(agents: &[Provider]) -> (BTreeSet<Provider>, Vec<Action>) {
        let home = Path::new(if cfg!(windows) { r"C:\h" } else { "/h" });
        let env = Env::at(home);
        let install = |provider, path: PathBuf| SkillInstall {
            provider,
            path,
            shared: true,
            enabled: true,
        };
        let shared = env.agents_skills().join("s");
        let skill = Skill {
            name: "s".into(),
            title: "s".into(),
            description: String::new(),
            shared: Some(shared.clone()),
            source: Some("o/r".into()),
            installs: vec![
                install(Provider::Codex, shared),
                install(Provider::Claude, env.claude_skills().join("s")),
            ],
            default: false,
        };
        let change = Change::UninstallSkill(skill);
        let choices = change.choices(agents);
        let form = ChangeForm {
            env,
            change,
            agents: agents.to_vec(),
            providers: choices.clone(),
            delete_data: false,
        };
        (choices, form.steps().unwrap())
    }

    #[test]
    fn uninstall_offers_managed_agents_and_leaves_no_link_dangling() {
        let home = Env::at(Path::new(if cfg!(windows) { r"C:\h" } else { "/h" }));
        let shared = Action::Remove(home.agents_skills().join("s"));
        let link = Action::Remove(home.claude_skills().join("s"));

        // Claude isn't managed, but its link to the shared folder goes with the folder.
        let (choices, steps) = uninstall(&[Provider::Codex]);
        assert_eq!(choices, BTreeSet::from([Provider::Codex]));
        assert!(
            steps.contains(&shared) && steps.contains(&link),
            "{steps:?}"
        );

        let (choices, steps) = uninstall(&[Provider::Claude]);
        assert_eq!(choices, BTreeSet::from([Provider::Claude]));
        assert!(
            steps.contains(&shared) && steps.contains(&link),
            "{steps:?}"
        );
    }
}
