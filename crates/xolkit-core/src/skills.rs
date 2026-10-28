use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::config::{
    claude_disabled_skills, codex_disabled_skills, codex_skill_key, read_json, read_toml,
};
use crate::{Env, Provider};

#[derive(Clone, Debug)]
pub struct Skill {
    /// Directory name; the skill's identity.
    pub name: String,
    /// `name` from the SKILL.md frontmatter; Claude's `skillOverrides` is keyed by it.
    pub title: String,
    pub description: String,
    /// The shared copy in `~/.agents/skills`, if there is one.
    pub shared: Option<PathBuf>,
    /// Where it was installed from (`owner/repo`), per `~/.agents/.skill-lock.json`.
    pub source: Option<String>,
    /// Every place a provider reads this skill from. Codex can have two: the shared copy and
    /// one in `~/.codex/skills`.
    pub installs: Vec<SkillInstall>,
    /// Ships with the agent (Codex's `~/.codex/skills/.system`): it cannot be removed or turned
    /// off. Its name is `.system/<dir>`, so it never merges with a user skill of the same name.
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillInstall {
    pub provider: Provider,
    /// The directory or symlink the provider reads.
    pub path: PathBuf,
    /// The provider reads the shared copy, directly or through a link, or reads a copy of it
    /// that `npx skills` made where it could not link.
    pub shared: bool,
    pub enabled: bool,
}

impl Skill {
    pub fn skill_md(&self) -> Option<PathBuf> {
        let dir = self
            .shared
            .as_ref()
            .or(self.installs.first().map(|i| &i.path))?;
        Some(dir.join("SKILL.md"))
    }

    /// `None` when the provider does not have the skill; otherwise whether any of its copies is on.
    pub fn enabled(&self, provider: Provider) -> Option<bool> {
        let mut installs = self
            .installs
            .iter()
            .filter(|i| i.provider == provider)
            .peekable();
        installs.peek()?;
        Some(installs.any(|i| i.enabled))
    }
}

pub fn scan_skills(env: &Env) -> Result<Vec<Skill>> {
    let lock = read_json(&env.skill_lock())?;
    let claude_off = claude_disabled_skills(&read_json(&env.claude_settings())?);
    let codex_off = codex_disabled_skills(&read_toml(&env.codex_config())?);
    let codex_enabled = |dir: &Path| !codex_off.contains(&codex_skill_key(&dir.join("SKILL.md")));
    // A folder of a locked skill's name in ~/.claude/skills is the copy `npx skills` makes where
    // it can't link, so it is as managed as a link.
    let managed_copy = |name: &str, path: &Path| {
        lock["skills"].get(name).is_some() && fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
    };

    let mut skills: BTreeMap<String, Skill> = BTreeMap::new();

    // Codex reads ~/.agents/skills itself.
    for (name, dir) in skill_dirs(&env.agents_skills()) {
        let skill = skills
            .entry(name.clone())
            .or_insert_with(|| new_skill(&name, &dir));
        skill.shared = Some(dir.clone());
        skill.source = lock["skills"][&name]["source"].as_str().map(str::to_owned);
        let enabled = codex_enabled(&dir);
        skill.installs.push(SkillInstall {
            provider: Provider::Codex,
            path: dir,
            shared: true,
            enabled,
        });
    }

    for (name, path) in skill_dirs(&env.claude_skills()) {
        let skill = skills
            .entry(name.clone())
            .or_insert_with(|| new_skill(&name, &path));
        let shared = skill
            .shared
            .as_deref()
            .is_some_and(|dir| same_dir(&path, dir) || managed_copy(&name, &path));
        let enabled = !claude_off.contains(&skill.title);
        skill.installs.push(SkillInstall {
            provider: Provider::Claude,
            path,
            shared,
            enabled,
        });
    }

    // Codex's own skills directory, next to the shared one.
    for (name, path) in skill_dirs(&env.codex_skills()) {
        let skill = skills
            .entry(name.clone())
            .or_insert_with(|| new_skill(&name, &path));
        let shared = skill
            .shared
            .as_deref()
            .is_some_and(|dir| same_dir(&path, dir));
        let enabled = codex_enabled(&path);
        skill.installs.push(SkillInstall {
            provider: Provider::Codex,
            path,
            shared,
            enabled,
        });
    }

    let mut skills: Vec<Skill> = skills.into_values().collect();
    for (dir, path) in skill_dirs(&env.codex_skills().join(".system")) {
        let mut skill = new_skill(&format!(".system/{dir}"), &path);
        skill.default = true;
        skill.installs.push(SkillInstall {
            provider: Provider::Codex,
            path,
            shared: false,
            enabled: true,
        });
        skills.push(skill);
    }
    Ok(skills)
}

/// Visible subdirectories (or links to them) that hold a SKILL.md, sorted by name.
fn skill_dirs(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, path)| !name.starts_with('.') && path.join("SKILL.md").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn new_skill(name: &str, dir: &Path) -> Skill {
    let text = fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
    let front = frontmatter(&text);
    Skill {
        name: name.to_owned(),
        title: front.name.unwrap_or_else(|| name.to_owned()),
        description: front.description.unwrap_or_default(),
        shared: None,
        source: None,
        installs: Vec::new(),
        default: false,
    }
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub(crate) struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// `name` and `description` from a SKILL.md. Falls back to a line scan, because many skills
/// ship frontmatter that is not valid YAML (e.g. unquoted colons in the description).
pub(crate) fn frontmatter(text: &str) -> Frontmatter {
    // Windows PowerShell 5.1 and some Windows editors start UTF-8 files with a byte order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.strip_prefix("---") else {
        return Frontmatter::default();
    };
    let block = rest.split("\n---").next().unwrap_or_default();
    if let Ok(front) = serde_yaml_ng::from_str::<Frontmatter>(block) {
        return front;
    }
    let field = |key: &str| {
        block.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
            Some(value.trim_matches(['"', '\'']).to_owned())
        })
    };
    Frontmatter {
        name: field("name"),
        description: field("description"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::link_dir;

    fn write_skill(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Does {name}.\n---\n\nBody\n"),
        )
        .unwrap();
    }

    #[test]
    fn frontmatter_parses_yaml_and_falls_back() {
        assert_eq!(
            frontmatter("---\nname: a\ndescription: >\n  folded\n  text\n---\nbody"),
            Frontmatter {
                name: Some("a".into()),
                description: Some("folded text".into())
            }
        );
        assert_eq!(
            frontmatter("---\nname: b\ndescription: Use when: things\n---\n"),
            Frontmatter {
                name: Some("b".into()),
                description: Some("Use when: things".into())
            }
        );
        assert_eq!(frontmatter("no frontmatter"), Frontmatter::default());
    }

    #[test]
    fn frontmatter_after_a_byte_order_mark_and_crlf() {
        let expected = Frontmatter {
            name: Some("a".into()),
            description: Some("Does a: well.".into()),
        };
        for text in [
            "\u{feff}---\nname: a\ndescription: \"Does a: well.\"\n---\n",
            "---\r\nname: a\r\ndescription: \"Does a: well.\"\r\n---\r\nbody\r\n",
            "\u{feff}---\r\nname: a\r\ndescription: Does a: well.\r\n---\r\n",
        ] {
            assert_eq!(frontmatter(text), expected, "{text:?}");
        }
    }

    #[test]
    fn scan_classifies_installs() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let agents = env.agents_skills();
        // Shared skill, linked into Claude the way `npx skills` does it.
        write_skill(&agents.join("shared"), "shared");
        fs::create_dir_all(env.claude_skills()).unwrap();
        link_dir(
            "../../.agents/skills/shared",
            env.claude_skills().join("shared"),
        );
        // Claude-only real directory, and a link to somewhere else (a dotfiles repo).
        write_skill(&env.claude_skills().join("local"), "local");
        write_skill(&home.path().join("dotfiles/foreign"), "foreign");
        link_dir(
            home.path().join("dotfiles/foreign"),
            env.claude_skills().join("foreign"),
        );
        // Codex's own dir: a skill only there, a second copy of a shared one, and Codex's
        // bundled skills, listed last as defaults.
        write_skill(&env.codex_skills().join("old"), "old");
        write_skill(&env.codex_skills().join("shared"), "shared");
        write_skill(&env.codex_skills().join(".system/bundled"), "bundled");
        // Disabled: shared for Codex, local for Claude.
        fs::create_dir_all(&env.codex_dir).unwrap();
        fs::write(
            env.codex_config(),
            format!(
                "[[skills.config]]\npath = '{}'\nenabled = false\n",
                agents.join("shared/SKILL.md").display()
            ),
        )
        .unwrap();
        fs::write(
            env.claude_settings(),
            r#"{"skillOverrides":{"local":"off"}}"#,
        )
        .unwrap();
        fs::write(
            env.skill_lock(),
            r#"{"version":3,"skills":{"shared":{"source":"o/r"}}}"#,
        )
        .unwrap();

        let skills = scan_skills(&env).unwrap();
        let get = |n: &str| skills.iter().find(|s| s.name == n).unwrap();
        assert_eq!(
            skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["foreign", "local", "old", "shared", ".system/bundled"]
        );
        let bundled = get(".system/bundled");
        assert!(bundled.default && !get("old").default);
        assert_eq!(bundled.title, "bundled");

        let shared = get("shared");
        assert_eq!(shared.source.as_deref(), Some("o/r"));
        assert_eq!(shared.description, "Does shared.");
        let summary: Vec<_> = shared
            .installs
            .iter()
            .map(|i| (i.provider, i.shared, i.enabled))
            .collect();
        assert_eq!(
            summary,
            [
                (Provider::Codex, true, false),
                (Provider::Claude, true, true),
                (Provider::Codex, false, true),
            ]
        );
        assert_eq!(shared.enabled(Provider::Codex), Some(true));

        assert_eq!(get("local").enabled(Provider::Claude), Some(false));
        assert!(!get("foreign").installs[0].shared);
        assert!(get("foreign").shared.is_none());
        assert_eq!(get("old").enabled(Provider::Claude), None);
        assert_eq!(get("old").enabled(Provider::Codex), Some(true));
    }

    #[test]
    fn scan_counts_a_copy_of_a_locked_skill_as_shared() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        // `npx skills` copies where it can't link; only "locked" is in the lock file.
        for name in ["locked", "unlocked"] {
            write_skill(&env.agents_skills().join(name), name);
            write_skill(&env.claude_skills().join(name), name);
        }
        fs::write(env.skill_lock(), r#"{"version":3,"skills":{"locked":{}}}"#).unwrap();
        let skills = scan_skills(&env).unwrap();
        let claude_shared = |n: &str| {
            let skill = skills.iter().find(|s| s.name == n).unwrap();
            let claude = skill
                .installs
                .iter()
                .find(|i| i.provider == Provider::Claude);
            claude.unwrap().shared
        };
        assert!(claude_shared("locked"));
        assert!(!claude_shared("unlocked"));
    }
}
