//! Settings in `~/.config/xolkit/` (`%APPDATA%\xolkit\` on Windows): `settings.json` (with the
//! profiles, see [`crate::profiles`]), plus the GitHub token in a private file of its own, so
//! `settings.json` can live in a dotfiles repository without leaking it.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use xolkit_core::{Env, Provider, gh_token};

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|h| h.join("AppData").join("Roaming")));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|h| h.join(".config")));
    base.unwrap_or_default().join("xolkit")
}

/// Moves the settings of skillshelf, as the app was called before 2.0, to where xolkit keeps them,
/// the first time xolkit starts. The saved theme follows the rename of the app's own themes.
pub fn migrate() {
    let new = config_dir();
    let old = new.with_file_name("skillshelf");
    match move_dir(&old, &new) {
        Ok(false) => return,
        Ok(true) => {}
        Err(e) => {
            eprintln!(
                "xolkit: could not move {} to {}: {e:#}",
                old.display(),
                new.display()
            );
            return;
        }
    }
    if let Some(theme) = get("theme").and_then(|name| renamed_theme(&name))
        && let Err(e) = set("theme", theme)
    {
        eprintln!("xolkit: could not save the theme: {e:#}");
    }
}

/// Renames `old` to `new` if only `old` is there; true if it did.
fn move_dir(old: &Path, new: &Path) -> Result<bool> {
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    fs::rename(old, new)?;
    Ok(true)
}

fn renamed_theme(name: &str) -> Option<String> {
    name.strip_prefix("skillshelf ")
        .map(|mode| format!("xolkit {mode}"))
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn token_path() -> PathBuf {
    config_dir().join("github-token")
}

pub fn load() -> Value {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn get(key: &str) -> Option<String> {
    load()[key].as_str().map(str::to_owned)
}

pub fn set(key: &str, value: impl Into<Value>) -> Result<()> {
    let value = value.into();
    update(|settings| settings[key] = value)
}

/// Reads `settings.json`, edits it and writes it back.
pub fn update(edit: impl FnOnce(&mut Value)) -> Result<()> {
    let mut settings = load();
    edit(&mut settings);
    fs::create_dir_all(config_dir())?;
    fs::write(
        settings_path(),
        serde_json::to_string_pretty(&settings)? + "\n",
    )?;
    Ok(())
}

/// The agents in a saved list of tool names, in display order: none until the user turns some on.
pub fn agents_in(saved: &Value) -> Vec<Provider> {
    let names: Vec<&str> = saved
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Provider::ALL
        .into_iter()
        .filter(|p| names.contains(&p.cli()))
        .collect()
}

/// Where GitHub downloads get a token from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubAccess {
    GhCli,
    Token,
    Anonymous,
}

impl GitHubAccess {
    /// In display order; the token form sits under the last one.
    pub const ALL: [GitHubAccess; 3] = [
        GitHubAccess::GhCli,
        GitHubAccess::Anonymous,
        GitHubAccess::Token,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GitHubAccess::GhCli => "GitHub CLI (gh)",
            GitHubAccess::Token => "Personal access token",
            GitHubAccess::Anonymous => "No token",
        }
    }

    fn key(self) -> &'static str {
        match self {
            GitHubAccess::GhCli => "gh",
            GitHubAccess::Token => "token",
            GitHubAccess::Anonymous => "none",
        }
    }

    /// The saved choice; the GitHub CLI when nothing is saved.
    pub fn load() -> GitHubAccess {
        let saved = get("github");
        GitHubAccess::ALL
            .into_iter()
            .find(|a| Some(a.key()) == saved.as_deref())
            .unwrap_or(GitHubAccess::GhCli)
    }

    pub fn save(self) -> Result<()> {
        set("github", self.key())
    }

    /// The token to send, if any. Runs `gh` for the CLI choice, so call it off the UI thread.
    pub fn token(self, env: &Env) -> Result<Option<String>> {
        match self {
            GitHubAccess::GhCli => gh_token(env).map(Some),
            GitHubAccess::Token => read_token().map(Some).context("no token is saved"),
            GitHubAccess::Anonymous => Ok(None),
        }
    }
}

pub fn read_token() -> Option<String> {
    let token = fs::read_to_string(token_path()).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Saves the token readable by the owner only. On Windows the file inherits the permissions of
/// `%APPDATA%`, which only the user (and administrators) can read.
pub fn save_token(token: &str) -> Result<()> {
    let token = token.trim();
    ensure!(!token.is_empty(), "the token is empty");
    ensure!(
        token.chars().all(|c| c.is_ascii_graphic()),
        "the token has characters GitHub tokens never contain"
    );
    fs::create_dir_all(config_dir())?;
    write_private(&token_path(), token.as_bytes()).context("could not save the token")
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        // An existing file keeps its old mode on open; tighten it before writing the secret.
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

pub fn delete_token() -> Result<()> {
    match fs::remove_file(token_path()) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use xolkit_core::Provider;

    #[test]
    fn agents_load_in_display_order_and_default_to_none() {
        let agents = |saved| super::agents_in(&saved);
        assert_eq!(agents(json!(null)), []);
        assert_eq!(agents(json!("claude")), []);
        assert_eq!(
            agents(json!(["codex", "claude", "gemini"])),
            [Provider::Claude, Provider::Codex]
        );
    }

    #[test]
    fn old_settings_move_once() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("skillshelf"), dir.path().join("xolkit"));
        assert!(!super::move_dir(&old, &new).unwrap());
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        assert!(super::move_dir(&old, &new).unwrap());
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("settings.json")).unwrap(),
            "{}"
        );
        // A later skillshelf folder is left alone once xolkit has its own.
        std::fs::create_dir(&old).unwrap();
        assert!(!super::move_dir(&old, &new).unwrap());
        assert!(old.exists());
    }

    #[test]
    fn own_themes_follow_the_rename() {
        assert_eq!(
            super::renamed_theme("skillshelf Dark").as_deref(),
            Some("xolkit Dark")
        );
        assert_eq!(super::renamed_theme("Nord"), None);
    }

    #[cfg(windows)]
    #[test]
    fn settings_live_in_appdata() {
        let appdata = std::env::var_os("APPDATA").unwrap();
        assert_eq!(
            super::config_dir(),
            std::path::Path::new(&appdata).join("xolkit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_file_is_owner_only() {
        use super::write_private;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
