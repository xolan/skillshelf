//! The Settings page: which agents skillshelf manages, and how GitHub downloads authenticate.

use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement,
    spinner::Spinner,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use skillshelf_core::{Provider, github_rate_limit};

use super::{Page, Shelf, divider, icon_line, icon_row};
use crate::settings::{self, GitHubAccess};
use crate::theme;

const NEW_TOKEN_URL: &str = "https://github.com/settings/personal-access-tokens/new";

impl Shelf {
    /// Asks GitHub for the request limit the current choice gets, which also proves the token works.
    pub(super) fn check_access(&mut self, cx: &mut Context<Self>) {
        let Some(env) = self.env.clone() else {
            return;
        };
        let access = self.github_access;
        self.access_status = None;
        self.access_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let token = access.token(&env)?;
                    let (limit, remaining) = github_rate_limit(token.as_deref())?;
                    anyhow::Ok(format!(
                        "GitHub allows {} requests an hour with this choice; {} are left.",
                        thousands(limit),
                        thousands(remaining)
                    ))
                })
                .await;
            this.update(cx, |this, cx| {
                this.access_status =
                    Some(result.map(Into::into).map_err(|e| format!("{e:#}").into()));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Turns managing an agent on or off in a profile, then reads the lists again for the new set.
    fn set_agent(
        &mut self,
        profile: usize,
        agent: Provider,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.edit_profile(profile, |p| p.set_agent(agent, on), window, cx) {
            return;
        }
        if profile == self.profiles.active {
            self.offers = None;
            self.offer_errors.clear();
            self.sync_lists();
        }
        self.reload(cx);
    }

    /// A switch for each agent, turning managing it on or off in a profile.
    pub(super) fn agent_rows(&self, profile: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let muted = cx.theme().muted_foreground;
        let agents = &self.profiles.list[profile].agents;
        Provider::ALL
            .into_iter()
            .map(|agent| {
                let on = agents.contains(&agent);
                let found = self.found.get(&agent);
                let note = match (&self.env, found) {
                    (None, _) => "Looking for it…".to_owned(),
                    (Some(env), Some(place)) => format!("Found at {}", env.tilde(place)),
                    (Some(_), None) => "Not found on this computer".to_owned(),
                };
                h_flex()
                    .gap_3()
                    .child(
                        Switch::new(SharedString::from(format!(
                            "agent-{profile}-{}",
                            agent.cli()
                        )))
                        .checked(on)
                        // An agent that isn't installed can be turned off, not on.
                        .disabled(self.busy || (!on && found.is_none()))
                        .on_change(cx.listener(
                            move |this, on: &bool, window, cx| {
                                this.set_agent(profile, agent, *on, window, cx)
                            },
                        )),
                    )
                    .child(
                        v_flex()
                            .child(div().text_sm().child(agent.name()))
                            .child(div().text_xs().text_color(muted).child(note)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn set_access(&mut self, access: GitHubAccess, window: &mut Window, cx: &mut Context<Self>) {
        self.github_access = access;
        if let Err(e) = access.save() {
            window.push_notification(Notification::error(format!("{e:#}")), cx);
        }
        self.check_access(cx);
    }

    fn save_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let token = self.token_input.read(cx).value().to_string();
        match settings::save_token(&token) {
            Ok(()) => {
                self.has_token = true;
                self.token_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                window.push_notification(Notification::success("Token saved"), cx);
                self.set_access(GitHubAccess::Token, window, cx);
            }
            Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
        }
    }

    fn remove_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match settings::delete_token() {
            Ok(()) => {
                self.has_token = false;
                window.push_notification(Notification::success("Token removed"), cx);
                self.check_access(cx);
            }
            Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
        }
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_theme = cx.theme().theme_name().clone();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let selected = GitHubAccess::ALL
            .iter()
            .position(|a| *a == self.github_access);
        let status = match &self.access_status {
            None => icon_row(Spinner::new().small(), "Checking GitHub access…").text_color(muted),
            Some(Ok(text)) => {
                icon_row(IconName::CircleCheck, text.clone()).text_color(theme.success)
            }
            Some(Err(error)) => {
                icon_row(IconName::CircleAlert, error.clone()).text_color(theme.danger)
            }
        };
        let token_place = self
            .env
            .as_ref()
            .map(|env| env.tilde(&settings::config_dir().join("github-token")))
            .unwrap_or_default();
        let token_note = if self.has_token {
            format!("A token is saved in {token_place}, readable only by you.")
        } else {
            "Use a fine-grained token with read-only access to public repositories.".to_owned()
        };
        let token_form =
            v_flex()
                .gap_2()
                .pl_6()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .w(px(340.))
                                .child(Input::new(&self.token_input).small().mask_toggle()),
                        )
                        .child(
                            Button::new("save-token")
                                .small()
                                .primary()
                                .label("Save token")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_token(window, cx)),
                                ),
                        )
                        .when(self.has_token, |row| {
                            row.child(
                                Button::new("remove-token")
                                    .small()
                                    .ghost()
                                    .label("Remove")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.remove_token(window, cx)
                                    })),
                            )
                        }),
                )
                .child(div().text_xs().text_color(muted).child(token_note))
                .child(
                    h_flex().child(
                        Button::new("new-token")
                            .small()
                            .link()
                            .label("Create a token on GitHub")
                            .on_click(|_, _, cx| cx.open_url(NEW_TOKEN_URL)),
                    ),
                );
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_lg().font_semibold().child("Settings"))
                    .child(
                        Button::new("reload")
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Reload")
                            .tooltip("Read skills and plugins again (Ctrl+R)")
                            .on_click(cx.listener(|this, _, _, cx| this.reload_all(cx))),
                    ),
            )
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .p_4()
                            .gap_3()
                            .max_w(px(680.))
                            .child(div().font_semibold().child("Agents"))
                            .child(div().text_sm().text_color(muted).child(format!(
                                "skillshelf lists and changes skills and plugins only for the \
                                 agents turned on here, and leaves the others alone. The choice \
                                 belongs to the “{}” profile.",
                                self.profiles.active().name
                            )))
                            .child(
                                v_flex()
                                    .gap_3()
                                    .children(self.agent_rows(self.profiles.active, cx)),
                            )
                            .child(divider(cx))
                            .child(div().font_semibold().child("GitHub access"))
                            .child(div().text_sm().text_color(muted).child(
                                "Skills download from GitHub. Without a token GitHub allows 60 \
                                 requests an hour, and each download uses two or three.",
                            ))
                            .child(
                                RadioGroup::vertical("github-access")
                                    // The label's own line, not the radio's 17 px one, so it
                                    // lines up with the circle.
                                    .children(GitHubAccess::ALL.map(|access| {
                                        let label = access.label();
                                        Radio::new(label)
                                            .accessibility_label(label)
                                            .child(div().line_height(icon_line()).child(label))
                                    }))
                                    .selected_index(selected)
                                    .on_change(cx.listener(|this, ix: &usize, window, cx| {
                                        this.set_access(GitHubAccess::ALL[*ix], window, cx)
                                    })),
                            )
                            .when(self.github_access == GitHubAccess::Token, |this| {
                                this.child(token_form)
                            })
                            .child(div().text_sm().child(status))
                            .child(divider(cx))
                            .child(div().font_semibold().child("Theme"))
                            .child(div().text_sm().text_color(muted).child(
                                "Base16 schemes dropped into the themes folder show up here too.",
                            ))
                            .child(
                                h_flex().child(
                                    Button::new("theme")
                                        .small()
                                        .outline()
                                        .icon(IconName::Palette)
                                        .label(active_theme.clone())
                                        .dropdown_menu(move |menu, _, cx| {
                                            let mut menu = menu.scrollable(true).max_h(px(420.));
                                            for (name, _) in theme::names(cx) {
                                                let checked = name == active_theme;
                                                let pick = name.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(name)
                                                        .checked(checked)
                                                        .on_click(move |_, _, cx| {
                                                            theme::select(&pick, cx)
                                                        }),
                                                );
                                            }
                                            menu
                                        }),
                                ),
                            )
                            .child(divider(cx))
                            .child(div().font_semibold().child("About"))
                            .child(div().text_sm().text_color(muted).child(format!(
                                "skillshelf {}, released under the MIT License.",
                                env!("CARGO_PKG_VERSION")
                            )))
                            .child(
                                h_flex().child(
                                    Button::new("licenses")
                                        .small()
                                        .icon(IconName::FileText)
                                        .label("Third-party licenses")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_page(Page::Licenses, window, cx)
                                        })),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// `5000` as `5,000`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(60), "60");
        assert_eq!(thousands(5000), "5,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
