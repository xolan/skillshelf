//! Reading and editing provider config files without disturbing their formatting.

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

pub(crate) fn read_json(path: &Path) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

pub(crate) fn write_json(path: &Path, json: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(json)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

pub(crate) fn read_toml(path: &Path) -> Result<DocumentMut> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("{} is not valid TOML", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

pub(crate) fn write_toml(path: &Path, doc: &DocumentMut) -> Result<()> {
    write_atomic(path, doc.to_string().as_bytes())
}

/// Replaces a file in one step, keeping its permissions. Writes through symlinks, so files
/// managed by a dotfile tool stay links.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = canonical(path);
    let dir = path
        .parent()
        .context("config file has no parent directory")?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().context("config file has no name")?;
    let tmp = dir.join(format!(".{}.skillshelf-tmp", name.to_string_lossy()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Start private: config files can hold secrets.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("could not write {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Ok(meta) = fs::metadata(&path) {
        fs::set_permissions(&tmp, meta.permissions())?;
    }
    fs::rename(&tmp, &path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        replace_error(error, &path)
    })
}

/// Windows answers "Access is denied" both for a read-only file and for one another program
/// holds open without letting it be replaced (some editors do; Claude Code and Codex don't).
fn replace_error(error: std::io::Error, path: &Path) -> anyhow::Error {
    let shown = path.display();
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION
    if cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32)) {
        if fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
            return anyhow::anyhow!("could not replace {shown}: it is read-only");
        }
        return anyhow::anyhow!(
            "could not replace {shown}: another program has it open; close that program and try again"
        );
    }
    anyhow::Error::new(error).context(format!("could not replace {shown}"))
}

/// Skill names turned off in Claude's `skillOverrides`.
pub(crate) fn claude_disabled_skills(settings: &Value) -> HashSet<String> {
    settings
        .get("skillOverrides")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.as_str() == Some("off"))
        .map(|(k, _)| k.clone())
        .collect()
}

pub(crate) fn set_claude_skill(settings: &mut Value, name: &str, enabled: bool) -> Result<()> {
    let root = settings
        .as_object_mut()
        .context("settings.json is not a JSON object")?;
    let overrides = root
        .entry("skillOverrides")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("skillOverrides in settings.json is not an object")?;
    if !enabled {
        overrides.insert(name.to_owned(), "off".into());
    } else if overrides.get(name).and_then(Value::as_str) == Some("off") {
        overrides.shift_remove(name);
    }
    if overrides.is_empty() {
        root.shift_remove("skillOverrides");
    }
    Ok(())
}

/// `SKILL.md` paths turned off in Codex's `[[skills.config]]`, as [`codex_skill_key`]s.
pub(crate) fn codex_disabled_skills(doc: &DocumentMut) -> HashSet<String> {
    doc.get("skills")
        .and_then(|s| s.get("config"))
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flatten()
        .filter(|t| t.get("enabled").and_then(Item::as_bool) == Some(false))
        .filter_map(|t| t.get("path").and_then(Item::as_str))
        .map(|path| codex_skill_key(Path::new(path)))
        .collect()
}

/// What Codex compares `[[skills.config]]` paths by: both sides canonicalized, and on Windows
/// without regard to separators or ASCII case. So an entry still matches when another tool
/// spelled the path differently.
pub(crate) fn codex_skill_key(path: &Path) -> String {
    let key = canonical(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        key.replace('/', "\\").to_ascii_lowercase()
    } else {
        key
    }
}

/// The path Codex writes for a skill: on Windows canonical, as `dunce` makes it (backslashes, the
/// real case, links resolved, no `\\?\`). Elsewhere the path as given.
fn codex_skill_path(skill_md: &Path) -> String {
    if cfg!(windows) {
        canonical(skill_md).to_string_lossy().into_owned()
    } else {
        skill_md.to_string_lossy().into_owned()
    }
}

/// `path` with links resolved, or as given when it does not exist.
fn canonical(path: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    let resolved = dunce::canonicalize(path).unwrap_or_else(|_| dunce::simplified(path).into());
    #[cfg(not(windows))]
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved
}

pub(crate) fn set_codex_skill(doc: &mut DocumentMut, skill_md: &Path, enabled: bool) -> Result<()> {
    let skills = doc
        .entry("skills")
        .or_insert_with(implicit_table)
        .as_table_mut()
        .context("`skills` in config.toml is not a table")?;
    let configs = skills
        .entry("config")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .context("`skills.config` in config.toml is not an array of tables")?;
    let key = codex_skill_key(skill_md);
    configs.retain(|t| {
        t.get("path")
            .and_then(Item::as_str)
            .is_none_or(|path| codex_skill_key(Path::new(path)) != key)
    });
    if !enabled {
        let mut entry = Table::new();
        entry["path"] = value(codex_skill_path(skill_md));
        entry["enabled"] = value(false);
        configs.push(entry);
    }
    if configs.is_empty() {
        skills.remove("config");
    }
    if skills.is_empty() {
        doc.remove("skills");
    }
    Ok(())
}

pub(crate) fn set_codex_plugin(doc: &mut DocumentMut, id: &str, enabled: bool) -> Result<()> {
    let plugins = doc
        .entry("plugins")
        .or_insert_with(implicit_table)
        .as_table_mut()
        .context("`plugins` in config.toml is not a table")?;
    let plugin = plugins
        .entry(id)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .with_context(|| format!("`plugins.\"{id}\"` in config.toml is not a table"))?;
    plugin["enabled"] = value(enabled);
    Ok(())
}

/// Drops the hook trust entries Codex keeps for a plugin; `codex plugin remove` leaves them.
pub(crate) fn forget_codex_hooks(doc: &mut DocumentMut, id: &str) {
    let Some(state) = doc
        .get_mut("hooks")
        .and_then(|h| h.get_mut("state"))
        .and_then(Item::as_table_like_mut)
    else {
        return;
    };
    let prefix = format!("{id}:");
    let keys: Vec<String> = state
        .iter()
        .map(|(k, _)| k.to_owned())
        .filter(|k| k.starts_with(&prefix))
        .collect();
    for key in keys {
        state.remove(&key);
    }
}

fn implicit_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_skill_toggle_round_trips() {
        let mut settings: Value =
            serde_json::from_str(r#"{"model":"x","skillOverrides":{"a":"name-only"}}"#).unwrap();
        set_claude_skill(&mut settings, "b", false).unwrap();
        assert_eq!(
            claude_disabled_skills(&settings),
            HashSet::from(["b".to_owned()])
        );
        set_claude_skill(&mut settings, "b", true).unwrap();
        set_claude_skill(&mut settings, "a", true).unwrap();
        assert_eq!(
            settings.to_string(),
            r#"{"model":"x","skillOverrides":{"a":"name-only"}}"#
        );
    }

    #[test]
    fn codex_skill_toggle_keeps_other_config() {
        let original = "# mine\nmodel = \"x\"\n\n[tui]\nfoo = 1\n";
        let skill_md = Path::new("/h/.agents/skills/a/SKILL.md");
        let mut doc: DocumentMut = original.parse().unwrap();
        set_codex_skill(&mut doc, skill_md, false).unwrap();
        assert!(codex_disabled_skills(&doc).contains(&codex_skill_key(skill_md)));
        assert!(doc.to_string().contains("[[skills.config]]"));
        set_codex_skill(&mut doc, skill_md, true).unwrap();
        assert_eq!(doc.to_string(), original);
    }

    #[test]
    fn codex_skill_entries_match_like_codex_matches_them() {
        let dir = tempfile::tempdir().unwrap();
        let skill_md = dir.path().join("a").join("SKILL.md");
        fs::create_dir_all(dir.path().join("a")).unwrap();
        fs::write(&skill_md, "").unwrap();
        crate::testing::link_dir("a", dir.path().join("alias"));
        let mut spellings = vec![
            dir.path()
                .join("alias")
                .join("SKILL.md")
                .display()
                .to_string(),
        ];
        if cfg!(windows) {
            let plain = skill_md.display().to_string();
            spellings.push(plain.replace('\\', "/"));
            spellings.push(plain.to_lowercase());
        }
        for spelling in spellings {
            let entry = format!("[[skills.config]]\npath = '{spelling}'\nenabled = false\n");
            let mut doc: DocumentMut = entry.parse().unwrap();
            assert!(
                codex_disabled_skills(&doc).contains(&codex_skill_key(&skill_md)),
                "{spelling}"
            );
            set_codex_skill(&mut doc, &skill_md, true).unwrap();
            assert_eq!(doc.to_string(), "", "{spelling}");
        }

        // Windows gets the spelling Codex itself writes: canonical, with backslashes.
        let mut doc = DocumentMut::new();
        let mixed = dir.path().join("alias/SKILL.md");
        set_codex_skill(&mut doc, &mixed, false).unwrap();
        let written = doc["skills"]["config"][0]["path"]
            .as_str()
            .unwrap()
            .to_owned();
        if cfg!(windows) {
            let real = fs::canonicalize(&skill_md).unwrap().display().to_string();
            assert_eq!(Some(written.as_str()), real.strip_prefix(r"\\?\"));
        } else {
            assert_eq!(Path::new(&written), mixed);
        }
    }

    #[test]
    fn codex_plugin_toggle_and_hook_cleanup() {
        let mut doc: DocumentMut = "[plugins.\"p@m\"]\nenabled = true\n\n[hooks.state.\"p@m:hooks/hooks.json:pre:0:0\"]\ntrusted_hash = \"x\"\n\n[hooks.state.\"q@m:h:0\"]\ntrusted_hash = \"y\"\n"
            .parse()
            .unwrap();
        set_codex_plugin(&mut doc, "p@m", false).unwrap();
        forget_codex_hooks(&mut doc, "p@m");
        let text = doc.to_string();
        assert!(text.contains("[plugins.\"p@m\"]\nenabled = false"));
        assert!(!text.contains("p@m:hooks"));
        assert!(text.contains("q@m:h:0"));
    }

    #[test]
    fn atomic_write_keeps_permissions_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        fs::write(&real, "a = 1\n").unwrap();
        #[cfg(unix)]
        let link = dir.path().join("config.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
        }
        // File symlinks need Developer Mode on Windows; write the file itself there.
        #[cfg(not(unix))]
        let link = real.clone();
        write_atomic(&link, b"a = 2\n").unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                fs::metadata(&real).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn atomic_write_says_when_another_program_holds_the_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{}").unwrap();
        // Shares reading and writing but not deleting, like some editors.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&path)
            .unwrap();
        let error = write_atomic(&path, b"{\"a\":1}").unwrap_err().to_string();
        assert!(error.contains("another program has it open"), "{error}");
        assert!(!error.contains(r"\\?\"), "{error}");
        drop(held);
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "temporary file left"
        );
        write_atomic(&path, b"{\"a\":1}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    }
}
