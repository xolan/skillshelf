//! The Licenses page: one entry per bundled asset and Rust crate, with its license texts. The data
//! is `licenses.json`, which `mise run licenses` writes along with `THIRD_PARTY_LICENSES.md`.

use std::sync::LazyLock;

use gpui_kit::component::{
    ActiveTheme, button::Button, button::ButtonVariants, scroll::ScrollableElement, text::TextView,
    v_flex,
};
use gpui_kit::*;
use serde::Deserialize;

use super::{Selection, Shelf, list_body, matches};

const LICENSES: &str = include_str!("../../licenses.json");

#[derive(Deserialize)]
struct File {
    entries: Vec<FileEntry>,
    texts: Vec<Text>,
}

#[derive(Deserialize)]
struct FileEntry {
    name: String,
    version: String,
    license: String,
    link: String,
    texts: Vec<TextRef>,
}

/// One of an entry's license texts, with the entry's own copyright lines.
#[derive(Deserialize)]
struct TextRef {
    text: usize,
    notices: Vec<String>,
}

/// A license text, shared by every entry that ships the same one.
#[derive(Deserialize)]
struct Text {
    name: String,
    body: String,
}

pub(super) struct Entry {
    pub name: String,
    /// Empty for assets.
    pub version: String,
    /// The license the asset or crate declares, e.g. "MIT OR Apache-2.0".
    pub license: String,
    /// Its repository or website.
    pub link: String,
    /// Its license texts with its copyright lines, as Markdown.
    pub body: SharedString,
    /// Lowercased name, license, text names and copyright lines, for the filter.
    search: String,
}

pub(super) static ENTRIES: LazyLock<Vec<Entry>> = LazyLock::new(|| parse(LICENSES));

fn parse(json: &str) -> Vec<Entry> {
    let file: File =
        serde_json::from_str(json).expect("licenses.json is written by `mise run licenses`");
    file.entries
        .into_iter()
        .map(|entry| {
            let mut body = String::new();
            let mut search = format!("{} {}", entry.name, entry.license);
            if entry.texts.is_empty() {
                body.push_str(&format!(
                    "No license file ships with {}. Its license is {}, and the full text is at its source.",
                    entry.name, entry.license
                ));
            }
            for TextRef { text, notices } in &entry.texts {
                let text = &file.texts[*text];
                body.push_str(&format!("### {}\n\n", text.name));
                for notice in notices {
                    body.push_str(&format!("{notice}\n\n"));
                }
                body.push_str(&fence(&text.body));
                body.push_str("\n\n");
                search.push_str(&format!(" {} {}", text.name, notices.join(" ")));
            }
            Entry {
                body: body.trim_end().to_owned().into(),
                search: search.to_lowercase(),
                name: entry.name,
                version: entry.version,
                license: entry.license,
                link: entry.link,
            }
        })
        .collect()
}

/// A plain-text code block, fenced with more backticks than any run inside the text.
fn fence(text: &str) -> String {
    let longest = text
        .lines()
        .filter(|line| !line.is_empty() && line.chars().all(|c| c == '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let ticks = "`".repeat((longest + 1).max(3));
    format!("{ticks}text\n{text}\n{ticks}")
}

impl Shelf {
    /// Entries matching the filter, with their index in [`ENTRIES`].
    pub(super) fn visible_licenses(
        &self,
        cx: &App,
    ) -> impl Iterator<Item = (usize, &'static Entry)> + use<> {
        let filter = self.filter_text(cx);
        ENTRIES
            .iter()
            .enumerate()
            .filter(move |(_, e)| matches(&filter, &[&e.search]))
    }

    pub(super) fn render_licenses(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<_> = self
            .visible_licenses(cx)
            .map(|(ix, entry)| {
                let selection = Selection::License(ix);
                let selected = self.selected.as_ref() == Some(&selection);
                self.row(
                    format!("license-{ix}"),
                    selected,
                    entry.name.clone(),
                    (!entry.version.is_empty()).then_some(entry.version.as_str()),
                    entry.license.clone(),
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
        let entry = ENTRIES.get(ix)?;
        let link = entry.link.clone();
        let open_project = Button::new("license-link")
            .ghost()
            .label("Open project")
            .on_click(move |_, _, cx| cx.open_url(&link));
        let subtitle = match entry.version.as_str() {
            "" => entry.license.clone(),
            version => format!("{version} · {}", entry.license),
        };
        Some(
            v_flex()
                .size_full()
                .child(self.detail_header(entry.name.clone(), Some(subtitle), open_project, cx))
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
                                    TextView::markdown(("license-md", ix), entry.body.clone())
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
    use super::{ENTRIES, fence, parse};

    const FILE: &str = r###"{
        "entries": [
            {"name": "font", "version": "", "license": "OFL-1.1", "link": "https://a", "texts": [{"text": 1, "notices": []}]},
            {"name": "a", "version": "1.0", "license": "MIT OR Apache-2.0", "link": "https://b", "texts": [
                {"text": 0, "notices": ["Copyright 2020 A"]}, {"text": 1, "notices": []}
            ]},
            {"name": "b", "version": "2.0", "license": "Zlib", "link": "https://c", "texts": []}
        ],
        "texts": [
            {"name": "MIT License", "body": "Permission is hereby granted."},
            {"name": "CDLA Permissive 2.0", "body": "## 1. Provision of the Data\n\nText."}
        ]
    }"###;

    #[test]
    fn one_entry_per_thing_with_its_texts_and_copyright_lines() {
        let entries = parse(FILE);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["font", "a", "b"]);
        let a = &entries[1];
        assert!(
            a.body
                .starts_with("### MIT License\n\nCopyright 2020 A\n\n```text\nPermission")
        );
        // A heading inside a license text stays inside its code block.
        assert!(
            a.body
                .ends_with("```text\n## 1. Provision of the Data\n\nText.\n```")
        );
        assert!(a.search.contains("copyright 2020 a") && a.search.contains("cdla"));
        assert!(
            entries[2]
                .body
                .starts_with("No license file ships with b. Its license is Zlib")
        );
    }

    #[test]
    fn fences_outlast_backtick_runs_in_the_text() {
        assert!(fence("a\n````\nb").starts_with("`````text\n"));
        assert!(fence("a").starts_with("```text\n"));
    }

    #[test]
    fn bundled_file_lists_assets_then_crates() {
        assert_eq!(ENTRIES[0].name, "Geist and Geist Mono fonts");
        let roots = ENTRIES.iter().find(|e| e.name == "webpki-roots").unwrap();
        assert!(
            roots
                .body
                .starts_with("### CDLA Permissive 2.0\n\n```text\n")
        );
        assert!(roots.body.contains("\n## 1. Provision of the Data\n"));
        assert!(roots.body.ends_with("```"));
    }
}
