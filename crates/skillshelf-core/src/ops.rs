//! Changes are planned as a list of [`Action`]s, shown to the user, then executed.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use toml_edit::DocumentMut;

use crate::config::{
    forget_codex_hooks, read_json, read_toml, set_claude_skill, set_codex_plugin, set_codex_skill,
    write_json, write_toml,
};
use crate::discover::{add_lock_entry, staging_root};
use crate::{AvailablePlugin, Env, Plugin, Provider, Skill, SkillInstall};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Unlinks a symlink or junction, or deletes a file or directory. Never follows links.
    Remove(PathBuf),
    /// Links a folder: a symlink, or a junction on Windows.
    Link {
        link: PathBuf,
        target: PathBuf,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    SetClaudeSkill {
        title: String,
        enabled: bool,
    },
    SetCodexSkill {
        skill_md: PathBuf,
        enabled: bool,
    },
    SetCodexPlugin {
        id: String,
        enabled: bool,
    },
    ForgetCodexHooks {
        id: String,
    },
    ForgetLock {
        name: String,
    },
    AddLock {
        name: String,
        entry: Value,
    },
    Run {
        provider: Provider,
        args: Vec<String>,
    },
}

impl Action {
    /// One line for the confirm dialog.
    pub fn describe(&self, env: &Env) -> String {
        let show = |p: &Path| env.tilde(p);
        let on_off = |enabled: &bool| if *enabled { "on" } else { "off" };
        match self {
            Action::Remove(path) if path.starts_with(staging_root(env)) => {
                "Delete the old copy".to_owned()
            }
            Action::Remove(path) => match fs::symlink_metadata(path) {
                Ok(m) if m.is_symlink() => format!("Unlink {}", show(path)),
                _ => format!("Delete {}", show(path)),
            },
            Action::Link { link, target } => format!("Link {} → {}", show(link), show(target)),
            Action::SetClaudeSkill { title, enabled } => format!(
                "Turn {} skill \"{title}\" in {}",
                on_off(enabled),
                show(&env.claude_settings())
            ),
            Action::SetCodexSkill {
                skill_md,
                enabled: true,
            } => format!(
                "Remove the entry for {} from {}",
                show(skill_md),
                show(&env.codex_config())
            ),
            Action::SetCodexSkill {
                skill_md,
                enabled: false,
            } => format!(
                "Turn off {} in {}",
                show(skill_md),
                show(&env.codex_config())
            ),
            Action::SetCodexPlugin { id, enabled } => format!(
                "Turn {} plugin {id} in {}",
                on_off(enabled),
                show(&env.codex_config())
            ),
            Action::ForgetCodexHooks { id } => format!(
                "Remove hook trust entries for {id} from {}",
                show(&env.codex_config())
            ),
            Action::Rename { from, to } if from.starts_with(staging_root(env)) => {
                format!("Move the download to {}", show(to))
            }
            Action::Rename { from, .. } => format!("Move {} aside", show(from)),
            Action::ForgetLock { name } => {
                format!("Remove {name} from {}", show(&env.skill_lock()))
            }
            Action::AddLock { name, .. } => {
                format!("Record {name} in {}", show(&env.skill_lock()))
            }
            Action::Run { provider, args } => {
                format!("Run `{} {}`", provider.cli(), args.join(" "))
            }
        }
    }
}

/// Runs actions in order and stops at the first failure.
pub fn execute(env: &Env, actions: &[Action]) -> Result<()> {
    for action in actions {
        run(env, action).with_context(|| action.describe(env))?;
    }
    Ok(())
}

fn run(env: &Env, action: &Action) -> Result<()> {
    match action {
        Action::Remove(path) => remove(path),
        Action::Link { link, target } => {
            fs::create_dir_all(link.parent().context("link has no parent directory")?)?;
            link_dir(target, link)
        }
        Action::Rename { from, to } => {
            fs::create_dir_all(to.parent().context("target has no parent directory")?)?;
            Ok(fs::rename(from, to)?)
        }
        Action::SetClaudeSkill { title, enabled } => edit_json(&env.claude_settings(), |s| {
            set_claude_skill(s, title, *enabled)
        }),
        Action::SetCodexSkill { skill_md, enabled } => edit_toml(&env.codex_config(), |doc| {
            set_codex_skill(doc, &skill_md.to_string_lossy(), *enabled)
        }),
        Action::SetCodexPlugin { id, enabled } => edit_toml(&env.codex_config(), |doc| {
            set_codex_plugin(doc, id, *enabled)
        }),
        Action::ForgetCodexHooks { id } => edit_toml(&env.codex_config(), |doc| {
            forget_codex_hooks(doc, id);
            Ok(())
        }),
        Action::ForgetLock { name } => edit_json(&env.skill_lock(), |lock| {
            if let Some(skills) = lock.get_mut("skills").and_then(Value::as_object_mut) {
                skills.shift_remove(name);
            }
            Ok(())
        }),
        Action::AddLock { name, entry } => {
            edit_json(&env.skill_lock(), |lock| add_lock_entry(lock, name, entry))
        }
        Action::Run { provider, args } => env.run(provider.cli(), args).map(drop),
    }
}

fn remove(path: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    // symlink_metadata never reports a link as a directory, so links are unlinked, not followed.
    if meta.is_dir() {
        fs::remove_dir_all(path)?;
    } else if is_dir_link(&meta) {
        // Windows unlinks a junction or directory symlink like an empty directory.
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(windows)]
fn is_dir_link(meta: &fs::Metadata) -> bool {
    std::os::windows::fs::FileTypeExt::is_symlink_dir(&meta.file_type())
}

#[cfg(not(windows))]
fn is_dir_link(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn link_dir(target: &Path, link: &Path) -> Result<()> {
    Ok(std::os::unix::fs::symlink(target, link)?)
}

/// A junction, as `npx skills` makes on Windows: unlike a directory symlink, it needs neither
/// administrator rights nor Developer Mode. Its target must be absolute.
#[cfg(windows)]
pub(crate) fn link_dir(target: &Path, link: &Path) -> Result<()> {
    anyhow::ensure!(
        target.is_absolute(),
        "a junction needs an absolute target, not {}",
        target.display()
    );
    Ok(junction::create(target, link)?)
}

/// Rewrites a JSON file only when the edit changes it.
fn edit_json(path: &Path, edit: impl FnOnce(&mut Value) -> Result<()>) -> Result<()> {
    let before = read_json(path)?;
    let mut after = before.clone();
    edit(&mut after)?;
    if after != before {
        write_json(path, &after)?;
    }
    Ok(())
}

/// Rewrites a TOML file only when the edit changes it.
fn edit_toml(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let mut doc = read_toml(path)?;
    let before = doc.to_string();
    edit(&mut doc)?;
    if doc.to_string() != before {
        write_toml(path, &doc)?;
    }
    Ok(())
}

/// Turns a skill on or off for one provider. Turning a shared skill on for Claude links it
/// into `~/.claude/skills` if it is not there yet.
pub fn toggle_skill(env: &Env, skill: &Skill, provider: Provider, enabled: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    match provider {
        Provider::Claude => {
            if enabled && skill.enabled(Provider::Claude).is_none() {
                let Some(shared) = &skill.shared else {
                    return actions;
                };
                actions.push(Action::Link {
                    link: env.claude_skills().join(&skill.name),
                    target: claude_link_target(env, shared, &skill.name),
                });
            }
            actions.push(Action::SetClaudeSkill {
                title: skill.title.clone(),
                enabled,
            });
        }
        Provider::Codex => {
            for install in skill
                .installs
                .iter()
                .filter(|i| i.provider == Provider::Codex)
            {
                actions.push(Action::SetCodexSkill {
                    skill_md: install.path.join("SKILL.md"),
                    enabled,
                });
            }
        }
    }
    actions
}

/// Relative like `npx skills` makes it, unless `$CLAUDE_CONFIG_DIR` moved Claude's directory.
/// Windows junctions take absolute targets only, so there it is always the shared folder.
pub(crate) fn claude_link_target(env: &Env, shared: &Path, name: &str) -> PathBuf {
    if cfg!(unix)
        && env.claude_dir == env.home.join(".claude")
        && shared == env.agents_skills().join(name)
    {
        Path::new("../../.agents/skills").join(name)
    } else {
        shared.to_path_buf()
    }
}

/// Removes a skill from the given providers, with every copy, link and config entry they have
/// for it. The shared copy in `~/.agents/skills` goes only when no remaining provider reads it;
/// until then, removing it from Codex turns it off there instead, because Codex reads that
/// directory on its own.
pub fn uninstall_skill(skill: &Skill, providers: &[Provider]) -> Vec<Action> {
    let removing = |i: &SkillInstall| providers.contains(&i.provider);
    let is_shared_dir = |i: &SkillInstall| Some(&i.path) == skill.shared.as_ref();
    let keep_shared = skill.installs.iter().any(|i| i.shared && !removing(i));

    let mut actions = Vec::new();
    for install in skill.installs.iter().filter(|i| removing(i)) {
        if is_shared_dir(install) && keep_shared {
            if install.enabled {
                actions.push(Action::SetCodexSkill {
                    skill_md: install.path.join("SKILL.md"),
                    enabled: false,
                });
            }
            continue;
        }
        if !is_shared_dir(install) {
            actions.push(Action::Remove(install.path.clone()));
        }
        if !install.enabled {
            actions.push(match install.provider {
                Provider::Claude => Action::SetClaudeSkill {
                    title: skill.title.clone(),
                    enabled: true,
                },
                Provider::Codex => Action::SetCodexSkill {
                    skill_md: install.path.join("SKILL.md"),
                    enabled: true,
                },
            });
        }
    }
    if !keep_shared && let Some(dir) = &skill.shared {
        actions.push(Action::Remove(dir.clone()));
        if skill.source.is_some() {
            actions.push(Action::ForgetLock {
                name: skill.name.clone(),
            });
        }
    }
    actions
}

pub fn toggle_plugin(plugin: &Plugin, provider: Provider, enabled: bool) -> Vec<Action> {
    if !plugin.installs.contains_key(&provider) {
        return Vec::new();
    }
    vec![match provider {
        Provider::Claude => Action::Run {
            provider,
            args: vec![
                "plugin".into(),
                if enabled { "enable" } else { "disable" }.into(),
                plugin.id.clone(),
                "--scope".into(),
                "user".into(),
            ],
        },
        Provider::Codex => Action::SetCodexPlugin {
            id: plugin.id.clone(),
            enabled,
        },
    }]
}

/// Installs a marketplace plugin through each chosen provider's tool, where it is offered and
/// not installed yet.
pub fn install_plugin(plugin: &AvailablePlugin, providers: &[Provider]) -> Vec<Action> {
    providers
        .iter()
        .filter(|p| plugin.providers.get(p) == Some(&false))
        .map(|&provider| {
            let verb = match provider {
                Provider::Claude => "install",
                Provider::Codex => "add",
            };
            let mut args = vec!["plugin".into(), verb.into(), plugin.id.clone()];
            if provider == Provider::Claude {
                args.extend(["--scope".into(), "user".into()]);
            }
            Action::Run { provider, args }
        })
        .collect()
}

/// Uninstalls a plugin through each provider's tool, then removes what the tool leaves behind:
/// Claude keeps the plugin's files until its own cleanup runs, and Codex keeps hook trust entries.
pub fn uninstall_plugin(plugin: &Plugin, providers: &[Provider], delete_data: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    for (&provider, install) in plugin
        .installs
        .iter()
        .filter(|(p, _)| providers.contains(p))
    {
        match provider {
            Provider::Claude => {
                let mut args = vec![
                    "plugin".into(),
                    "uninstall".into(),
                    plugin.id.clone(),
                    "--scope".into(),
                    "user".into(),
                ];
                if !delete_data {
                    args.push("--keep-data".into());
                }
                actions.push(Action::Run { provider, args });
                // Running Claude sessions register in `.in_use`; leave their files to Claude.
                if let Some(path) = &install.path
                    && dir_is_empty(&path.join(".in_use"))
                {
                    actions.push(Action::Remove(path.clone()));
                }
            }
            Provider::Codex => {
                actions.push(Action::Run {
                    provider,
                    args: vec!["plugin".into(), "remove".into(), plugin.id.clone()],
                });
                actions.push(Action::ForgetCodexHooks {
                    id: plugin.id.clone(),
                });
            }
        }
    }
    actions
}

fn dir_is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).map_or(true, |mut entries| entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginInstall;
    use crate::scan_skills;
    use crate::testing::link_dir;
    use std::collections::BTreeMap;

    fn write_skill(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: s\ndescription: d\n---\n").unwrap();
    }

    /// A shared skill linked into Claude, off in Codex, plus a copy in ~/.codex/skills.
    fn fixture() -> (tempfile::TempDir, Env) {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        write_skill(&env.agents_skills().join("s"));
        fs::create_dir_all(env.claude_skills()).unwrap();
        link_dir("../../.agents/skills/s", env.claude_skills().join("s"));
        write_skill(&env.codex_skills().join("s"));
        fs::create_dir_all(&env.codex_dir).unwrap();
        fs::write(
            env.codex_config(),
            format!(
                "model = \"x\"\n\n[[skills.config]]\npath = '{}'\nenabled = false\n",
                env.agents_skills().join("s/SKILL.md").display()
            ),
        )
        .unwrap();
        fs::write(
            env.skill_lock(),
            r#"{"version":3,"skills":{"s":{"source":"o/r"}},"dismissed":{}}"#,
        )
        .unwrap();
        (home, env)
    }

    fn skill(env: &Env) -> Skill {
        scan_skills(env)
            .unwrap()
            .into_iter()
            .find(|s| s.name == "s")
            .unwrap()
    }

    #[test]
    fn uninstall_everywhere_removes_all_traces() {
        let (_home, env) = fixture();
        let actions = uninstall_skill(&skill(&env), &Provider::ALL);
        execute(&env, &actions).unwrap();
        assert!(!env.agents_skills().join("s").exists());
        assert!(fs::symlink_metadata(env.claude_skills().join("s")).is_err());
        assert!(!env.codex_skills().join("s").exists());
        assert_eq!(
            fs::read_to_string(env.codex_config()).unwrap(),
            "model = \"x\"\n"
        );
        assert_eq!(
            fs::read_to_string(env.skill_lock()).unwrap(),
            "{\n  \"version\": 3,\n  \"skills\": {},\n  \"dismissed\": {}\n}\n"
        );
        assert!(scan_skills(&env).unwrap().is_empty());
    }

    #[test]
    fn uninstall_from_claude_keeps_shared_copy() {
        let (_home, env) = fixture();
        let actions = uninstall_skill(&skill(&env), &[Provider::Claude]);
        assert_eq!(actions, [Action::Remove(env.claude_skills().join("s"))]);
    }

    #[test]
    fn uninstall_from_codex_keeps_what_claude_reads() {
        let (_home, env) = fixture();
        let actions = uninstall_skill(&skill(&env), &[Provider::Codex]);
        // The shared copy is already off for Codex, so only its own copy goes.
        assert_eq!(actions, [Action::Remove(env.codex_skills().join("s"))]);
    }

    #[test]
    fn uninstall_never_deletes_a_link_target() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let dotfiles = home.path().join("dotfiles/s");
        write_skill(&dotfiles);
        fs::create_dir_all(env.claude_skills()).unwrap();
        link_dir(&dotfiles, env.claude_skills().join("s"));
        execute(&env, &uninstall_skill(&skill(&env), &Provider::ALL)).unwrap();
        assert!(dotfiles.join("SKILL.md").is_file());
        assert!(fs::symlink_metadata(env.claude_skills().join("s")).is_err());
    }

    #[test]
    fn remove_unlinks_a_folder_link_and_keeps_its_target() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_skill(outside.path());
        let link = home.path().join("s");
        link_dir(outside.path(), &link);
        let env = Env::at(home.path());
        assert!(
            Action::Remove(link.clone())
                .describe(&env)
                .starts_with("Unlink")
        );
        execute(&env, &[Action::Remove(link.clone())]).unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(outside.path().join("SKILL.md").is_file());
    }

    #[test]
    fn toggle_links_shared_skill_into_claude() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        write_skill(&env.agents_skills().join("s"));
        execute(
            &env,
            &toggle_skill(&env, &skill(&env), Provider::Claude, true),
        )
        .unwrap();
        let target = if cfg!(windows) {
            env.agents_skills().join("s")
        } else {
            "../../.agents/skills/s".into()
        };
        assert_eq!(
            fs::read_link(env.claude_skills().join("s")).unwrap(),
            target
        );
        assert_eq!(skill(&env).enabled(Provider::Claude), Some(true));
        // Nothing was turned off, so settings.json is not created.
        assert!(!env.claude_settings().exists());

        execute(
            &env,
            &toggle_skill(&env, &skill(&env), Provider::Claude, false),
        )
        .unwrap();
        assert_eq!(skill(&env).enabled(Provider::Claude), Some(false));
    }

    #[test]
    fn plugin_uninstall_sweeps_leftovers() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("cache/p/1.0");
        fs::create_dir_all(cache.join(".in_use")).unwrap();
        let install = |path: Option<PathBuf>| PluginInstall {
            version: "1.0".into(),
            path,
            enabled: true,
        };
        let plugin = Plugin {
            id: "p@m".into(),
            name: "p".into(),
            marketplace: "m".into(),
            description: String::new(),
            installs: BTreeMap::from([
                (Provider::Claude, install(Some(cache.clone()))),
                (Provider::Codex, install(None)),
            ]),
        };
        let run = |provider, args: &[&str]| Action::Run {
            provider,
            args: args.iter().map(|a| a.to_string()).collect(),
        };
        assert_eq!(
            uninstall_plugin(&plugin, &Provider::ALL, false),
            [
                run(
                    Provider::Claude,
                    &[
                        "plugin",
                        "uninstall",
                        "p@m",
                        "--scope",
                        "user",
                        "--keep-data"
                    ]
                ),
                Action::Remove(cache.clone()),
                run(Provider::Codex, &["plugin", "remove", "p@m"]),
                Action::ForgetCodexHooks { id: "p@m".into() },
            ]
        );
        // A running session holds the files: leave them to Claude.
        fs::write(cache.join(".in_use/1234"), "").unwrap();
        assert_eq!(
            uninstall_plugin(&plugin, &[Provider::Claude], true),
            [run(
                Provider::Claude,
                &["plugin", "uninstall", "p@m", "--scope", "user"]
            )]
        );
    }
}
