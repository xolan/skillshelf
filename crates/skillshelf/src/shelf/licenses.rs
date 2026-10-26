//! The Licenses page: `THIRD_PARTY_LICENSES.md`, one section per bundled asset list, crate list
//! and license text.

use std::sync::LazyLock;

use gpui_kit::component::{
    ActiveTheme, button::Button, button::ButtonVariants, scroll::ScrollableElement, text::TextView,
    v_flex,
};
use gpui_kit::*;

use super::{Selection, Shelf, list_body, matches};

const LICENSES: &str = include_str!("../../../../THIRD_PARTY_LICENSES.md");

pub(super) struct Section {
    pub title: String,
    /// One line for the list: who uses it, or what it covers.
    pub summary: String,
    /// The section's markdown, without its heading.
    pub body: SharedString,
    /// Lowercased body, for the filter.
    search: String,
}

pub(super) static SECTIONS: LazyLock<Vec<Section>> = LazyLock::new(|| parse(LICENSES));

/// Splits the file at its `##` and `###` headings, dropping headings with nothing under them.
fn parse(text: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        let heading = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "));
        match (heading, &mut current) {
            (Some(title), _) => {
                sections.extend(current.take());
                current = Some((title.to_owned(), String::new()));
            }
            (None, Some((_, body))) => {
                body.push_str(line);
                body.push('\n');
            }
            (None, None) => {}
        }
    }
    sections.extend(current);
    sections
        .into_iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(title, body)| {
            let summary = body
                .lines()
                .find(|l| !l.trim().is_empty() && !l.starts_with(['|', '`']))
                .unwrap_or_default()
                .trim_start_matches("Used by ")
                .to_owned();
            Section {
                title,
                summary,
                search: body.to_lowercase(),
                body: body.trim().to_owned().into(),
            }
        })
        .collect()
}

impl Shelf {
    /// Sections matching the filter, with their index in [`SECTIONS`].
    pub(super) fn visible_licenses(
        &self,
        cx: &App,
    ) -> impl Iterator<Item = (usize, &'static Section)> + use<> {
        let filter = self.filter_text(cx);
        SECTIONS
            .iter()
            .enumerate()
            .filter(move |(_, s)| matches(&filter, &[&s.title, &s.search]))
    }

    pub(super) fn render_licenses(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<_> = self
            .visible_licenses(cx)
            .map(|(ix, section)| {
                let selection = Selection::License(ix);
                let selected = self.selected.as_ref() == Some(&selection);
                self.row(
                    format!("license-{ix}"),
                    selected,
                    section.title.clone(),
                    None,
                    section.summary.clone(),
                    Vec::new(),
                    cx.listener(move |this, _, _, cx| this.select(Some(selection.clone()), cx)),
                    cx,
                )
            })
            .collect();
        v_flex()
            .size_full()
            .child(self.page_header("Licenses", Some(rows.len()), cx))
            .child(list_body("license-list", rows, &self.list_scroll, cx))
            .into_any_element()
    }

    pub(super) fn license_detail(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let section = SECTIONS.get(ix)?;
        let open_repo = Button::new("license-file")
            .ghost()
            .label("Open file")
            .on_click(|_, _, cx| {
                cx.open_url("https://github.com/xolan/skillshelf/blob/main/THIRD_PARTY_LICENSES.md")
            });
        Some(
            v_flex()
                .size_full()
                .child(self.detail_header(section.title.clone(), None, open_repo, cx))
                .child(
                    div()
                        .id(("license-detail", ix))
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(
                                    TextView::markdown(("license-md", ix), section.body.clone())
                                        .selectable(true),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{SECTIONS, parse};

    #[test]
    fn splits_on_headings_and_skips_empty_ones() {
        let sections = parse(
            "# Title\nintro\n## Assets\n| a | b |\nline\n## Texts\n\n### MIT License\n\nUsed by a 1, b 2.\n\ntext\n",
        );
        let titles: Vec<_> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Assets", "MIT License"]);
        assert_eq!(sections[0].summary, "line");
        assert_eq!(sections[1].summary, "a 1, b 2.");
        assert!(sections[1].body.ends_with("text"));
    }

    #[test]
    fn bundled_file_has_assets_crates_and_texts() {
        let titles: Vec<_> = SECTIONS.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles[..2], ["Bundled assets", "Rust crates"]);
        assert!(titles.contains(&"MIT License"));
    }
}
