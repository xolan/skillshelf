use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Where providers keep their files, and how to run their command-line tools.
#[derive(Clone, Debug)]
pub struct Env {
    pub home: PathBuf,
    /// `~/.claude`, or `$CLAUDE_CONFIG_DIR`.
    pub claude_dir: PathBuf,
    /// `~/.codex`, or `$CODEX_HOME`.
    pub codex_dir: PathBuf,
    /// `PATH` for provider tools: this process's `PATH`, then any extra entries from the user's
    /// login shell, because an app started from a desktop launcher gets a short `PATH`.
    pub path: Option<OsString>,
}

impl Env {
    pub fn detect() -> Result<Env> {
        let home = std::env::home_dir().context("no home directory")?;
        let dir_or = |var: &str, default: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(default))
        };
        Ok(Env {
            claude_dir: dir_or("CLAUDE_CONFIG_DIR", ".claude"),
            codex_dir: dir_or("CODEX_HOME", ".codex"),
            path: merge_paths(std::env::var_os("PATH"), login_shell_path()),
            home,
        })
    }

    /// An environment rooted at `home`, for tests.
    pub fn at(home: &Path) -> Env {
        Env {
            home: home.to_path_buf(),
            claude_dir: home.join(".claude"),
            codex_dir: home.join(".codex"),
            path: None,
        }
    }

    pub fn agents_skills(&self) -> PathBuf {
        self.home.join(".agents").join("skills")
    }

    pub fn skill_lock(&self) -> PathBuf {
        self.home.join(".agents").join(".skill-lock.json")
    }

    pub fn claude_skills(&self) -> PathBuf {
        self.claude_dir.join("skills")
    }

    pub fn claude_settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    pub fn codex_skills(&self) -> PathBuf {
        self.codex_dir.join("skills")
    }

    pub fn codex_config(&self) -> PathBuf {
        self.codex_dir.join("config.toml")
    }

    pub fn codex_plugin_cache(&self) -> PathBuf {
        self.codex_dir.join("plugins").join("cache")
    }

    /// `path` with the home directory shown as `~`: `~/…`, or `~\…` on Windows.
    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// Runs a provider tool and returns its stdout.
    pub fn run(&self, program: &str, args: &[String]) -> Result<String> {
        let mut command = Command::new(program);
        // Run from home: a project's tool config (e.g. a mise.toml pinning another codex) must
        // not leak into global changes.
        command
            .args(args)
            .current_dir(&self.home)
            .stdin(Stdio::null());
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        let output = command
            .output()
            .with_context(|| format!("could not run `{program}`"))?;
        if !output.status.success() {
            bail!(
                "`{program} {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// `first`, then the entries of `extra` that `first` lacks.
fn merge_paths(first: Option<OsString>, extra: Option<OsString>) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = first.iter().flat_map(std::env::split_paths).collect();
    for dir in extra.iter().flat_map(std::env::split_paths) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).ok().filter(|p| !p.is_empty())
}

/// `PATH` as the user's login shell sets it, or `None` if the shell fails or takes over 5 s.
fn login_shell_path() -> Option<OsString> {
    let shell = std::env::var_os("SHELL")?;
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", "env"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // shortcut: `env` output over the pipe buffer (64 KiB) stalls until the timeout; read it on a thread if that happens.
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))
        .map(OsString::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_the_platform_separator() {
        let home = if cfg!(windows) { r"C:\h" } else { "/h" };
        let env = Env::at(Path::new(home));
        let shown = env.tilde(&env.agents_skills().join("s"));
        if cfg!(windows) {
            assert_eq!(shown, r"~\.agents\skills\s");
            assert_eq!(
                env.skill_lock(),
                Path::new(r"C:\h\.agents\.skill-lock.json")
            );
            assert_eq!(
                env.codex_plugin_cache().to_str(),
                Some(r"C:\h\.codex\plugins\cache")
            );
        } else {
            assert_eq!(shown, "~/.agents/skills/s");
        }
        assert_eq!(env.tilde(Path::new("../x")), "../x");
    }

    #[test]
    fn merge_paths_keeps_own_order_first() {
        let merged = merge_paths(Some("/a:/b".into()), Some("/c:/a:/d".into()));
        assert_eq!(merged, Some("/a:/b:/c:/d".into()));
        assert_eq!(merge_paths(None, Some("/c".into())), Some("/c".into()));
        assert_eq!(merge_paths(None, None), None);
    }
}
