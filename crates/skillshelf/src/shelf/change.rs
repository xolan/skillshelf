//! The install and uninstall dialogs: which providers it applies to, and every step that will run.

use std::collections::BTreeSet;

use anyhow::Result;
use gpui_kit::component::{
    ActiveTheme, WindowExt, button::ButtonVariant, checkbox::Checkbox, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use skillshelf_core::{
    Action as Step, AvailablePlugin, Env, FetchedSkill, Plugin, Provider, Skill, install_plugin,
    install_skill, uninstall_plugin, uninstall_skill,
};

use super::Shelf;

pub(super) enum Change {
    UninstallSkill(Skill),
    UninstallPlugin(Plugin),
    InstallSkill(FetchedSkill),
    InstallPlugin(AvailablePlugin),
}

impl Change {
    fn name(&self) -> &str {
        match self {
            Change::UninstallSkill(skill) => &skill.name,
            Change::UninstallPlugin(plugin) => &plugin.id,
            Change::InstallSkill(skill) => &skill.name,
            Change::InstallPlugin(plugin) => &plugin.id,
        }
    }

    fn installs(&self) -> bool {
        matches!(self, Change::InstallSkill(_) | Change::InstallPlugin(_))
    }

    /// Providers the user can pick from.
    fn choices(&self) -> BTreeSet<Provider> {
        match self {
            Change::UninstallSkill(skill) => skill.installs.iter().map(|i| i.provider).collect(),
            Change::UninstallPlugin(plugin) => plugin.installs.keys().copied().collect(),
            Change::InstallSkill(_) => Provider::ALL.into(),
            Change::InstallPlugin(plugin) => plugin
                .providers
                .iter()
                .filter(|(_, installed)| !**installed)
                .map(|(p, _)| *p)
                .collect(),
        }
    }
}

pub(super) struct ChangeForm {
    env: Env,
    change: Change,
    providers: BTreeSet<Provider>,
    delete_data: bool,
}

impl ChangeForm {
    fn steps(&self) -> Result<Vec<Step>> {
        let providers: Vec<Provider> = self.providers.iter().copied().collect();
        Ok(match &self.change {
            Change::UninstallSkill(skill) => uninstall_skill(skill, &providers),
            Change::UninstallPlugin(plugin) => {
                uninstall_plugin(plugin, &providers, self.delete_data)
            }
            Change::InstallSkill(skill) => install_skill(&self.env, skill, &providers)?,
            Change::InstallPlugin(plugin) => install_plugin(plugin, &providers),
        })
    }
}

impl Render for ChangeForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let claude_plugin = matches!(&self.change,
            Change::UninstallPlugin(p) if p.installs.contains_key(&Provider::Claude));
        let steps = self.steps();
        let theme = cx.theme();
        let summary = match &steps {
            Ok(steps) if steps.is_empty() => v_flex().child("Pick at least one provider."),
            Ok(steps) => v_flex().children(steps.iter().map(|step| step.describe(&self.env))),
            Err(e) => v_flex().text_color(theme.danger).child(format!("{e:#}")),
        };
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_4()
                    .children(self.change.choices().into_iter().map(|provider| {
                        Checkbox::new(SharedString::from(format!("change-{}", provider.cli())))
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
    let installs_download = matches!(change, Change::InstallSkill(_));
    let (verb, done) = if installs {
        ("Install", format!("Installed {name}"))
    } else {
        ("Uninstall", format!("Uninstalled {name}"))
    };
    let title = format!("{verb} “{name}”?");
    let providers = change.choices();
    let form = cx.new(|_| ChangeForm {
        env,
        change,
        providers,
        delete_data: true,
    });
    let shelf = cx.entity();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (form, shelf, done) = (form.clone(), shelf.clone(), done.clone());
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
                    if installs_download {
                        // The install moves the download into place; keep it from being discarded.
                        shelf.fetched = None;
                    }
                    shelf.run(steps, Some(done.clone().into()), window, cx);
                });
                true
            })
    });
}
