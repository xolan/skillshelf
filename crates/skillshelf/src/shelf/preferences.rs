//! The Settings page: how GitHub downloads authenticate.

use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    notification::Notification,
    radio::RadioGroup,
    scroll::ScrollableElement,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use skillshelf_core::github_rate_limit;

use super::Shelf;
use crate::settings::{self, GitHubAccess};

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
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let selected = GitHubAccess::ALL
            .iter()
            .position(|a| *a == self.github_access);
        let status = match &self.access_status {
            None => h_flex()
                .gap_2()
                .text_color(muted)
                .child(Spinner::new().small())
                .child("Checking GitHub access…"),
            Some(Ok(text)) => h_flex()
                .gap_2()
                .text_color(theme.success)
                .child(IconName::CircleCheck)
                .child(text.clone()),
            Some(Err(error)) => h_flex()
                .gap_2()
                .items_start()
                .text_color(theme.danger)
                .child(div().pt_0p5().child(IconName::CircleAlert))
                .child(div().flex_1().min_w_0().child(error.clone())),
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
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_lg().font_semibold().child("Settings")),
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
                            .child(div().font_semibold().child("GitHub access"))
                            .child(div().text_sm().text_color(muted).child(
                                "Skills download from GitHub. Without a token GitHub allows 60 \
                                 requests an hour, and each download uses two or three.",
                            ))
                            .child(
                                RadioGroup::vertical("github-access")
                                    .children(GitHubAccess::ALL.map(GitHubAccess::label))
                                    .selected_index(selected)
                                    .on_change(cx.listener(|this, ix: &usize, window, cx| {
                                        this.set_access(GitHubAccess::ALL[*ix], window, cx)
                                    })),
                            )
                            .when(self.github_access == GitHubAccess::Token, |this| {
                                this.child(token_form)
                            })
                            .child(div().text_sm().child(status)),
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
