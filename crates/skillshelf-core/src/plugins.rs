use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{Env, Provider};

#[derive(Clone, Debug)]
pub struct Plugin {
    /// `name@marketplace`, as both providers spell it.
    pub id: String,
    pub name: String,
    pub marketplace: String,
    pub description: String,
    pub installs: BTreeMap<Provider, PluginInstall>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginInstall {
    pub version: String,
    /// The plugin's files, when they are on disk.
    pub path: Option<PathBuf>,
    pub enabled: bool,
    /// The provider installs it by default; it cannot be removed or turned off.
    pub default: bool,
}

impl Plugin {
    pub fn is_default(&self) -> bool {
        self.installs.values().any(|i| i.default)
    }
}

/// Installed user-scope plugins of the given providers. Each provider's tool is the source of
/// truth; a provider whose tool fails is reported in the error list, with its message, and skipped.
pub fn scan_plugins(env: &Env, providers: &[Provider]) -> (Vec<Plugin>, Vec<(Provider, String)>) {
    let mut plugins: BTreeMap<String, Plugin> = BTreeMap::new();
    let mut errors = Vec::new();
    for &provider in providers {
        let listed = env
            .run(
                provider.cli(),
                &["plugin".into(), "list".into(), "--json".into()],
            )
            .and_then(|json| match provider {
                Provider::Claude => parse_claude(&json),
                Provider::Codex => parse_codex(env, &json),
            });
        match listed {
            Ok(listed) => {
                for (id, install) in listed {
                    let plugin = plugins.entry(id.clone()).or_insert_with(|| new_plugin(&id));
                    if plugin.description.is_empty() {
                        plugin.description =
                            install.path.as_deref().map(description).unwrap_or_default();
                    }
                    plugin.installs.insert(provider, install);
                }
            }
            Err(e) => errors.push((provider, format!("{}: {e:#}", provider.label()))),
        }
    }
    (plugins.into_values().collect(), errors)
}

fn new_plugin(id: &str) -> Plugin {
    let (name, marketplace) = id.split_once('@').unwrap_or((id, ""));
    Plugin {
        id: id.to_owned(),
        name: name.to_owned(),
        marketplace: marketplace.to_owned(),
        description: String::new(),
        installs: BTreeMap::new(),
    }
}

/// `claude plugin list --json`: an array with one entry per installed scope.
fn parse_claude(json: &str) -> Result<Vec<(String, PluginInstall)>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        id: String,
        #[serde(default)]
        version: String,
        scope: String,
        enabled: bool,
        install_path: Option<PathBuf>,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(json).context("unexpected `claude plugin list` output")?;
    Ok(entries
        .into_iter()
        .filter(|e| e.scope == "user")
        .map(|e| {
            (
                e.id,
                PluginInstall {
                    version: e.version,
                    path: e.install_path,
                    enabled: e.enabled,
                    default: false,
                },
            )
        })
        .collect())
}

/// `codex plugin list --json`: `{ "installed": [...] }`. Files live in
/// `plugins/cache/<marketplace>/<name>/<version>`.
fn parse_codex(env: &Env, json: &str) -> Result<Vec<(String, PluginInstall)>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        plugin_id: String,
        name: String,
        marketplace_name: String,
        #[serde(default)]
        version: String,
        enabled: bool,
        #[serde(default)]
        install_policy: String,
    }
    #[derive(Deserialize)]
    struct List {
        installed: Vec<Entry>,
    }
    let list: List = serde_json::from_str(json).context("unexpected `codex plugin list` output")?;
    Ok(list
        .installed
        .into_iter()
        .map(|e| {
            let path = env
                .codex_plugin_cache()
                .join(&e.marketplace_name)
                .join(&e.name)
                .join(&e.version);
            let path = path.is_dir().then_some(path);
            (
                e.plugin_id,
                PluginInstall {
                    version: e.version,
                    path,
                    enabled: e.enabled,
                    default: e.install_policy == "INSTALLED_BY_DEFAULT",
                },
            )
        })
        .collect())
}

/// A plugin a configured marketplace offers.
#[derive(Clone, Debug, PartialEq)]
pub struct AvailablePlugin {
    pub id: String,
    pub name: String,
    pub marketplace: String,
    pub description: String,
    /// Installs across Claude users, when the marketplace reports it.
    pub install_count: Option<u64>,
    /// Providers whose marketplaces offer it, and whether it is installed there.
    pub providers: BTreeMap<Provider, bool>,
}

/// Every plugin the given providers' marketplaces offer, most installed first. A provider whose
/// tool fails is reported in the error list and skipped.
pub fn available_plugins(env: &Env, providers: &[Provider]) -> (Vec<AvailablePlugin>, Vec<String>) {
    let mut plugins: BTreeMap<String, AvailablePlugin> = BTreeMap::new();
    let mut errors = Vec::new();
    let args: Vec<String> = ["plugin", "list", "--available", "--json"]
        .map(String::from)
        .into();
    for &provider in providers {
        let listed = env
            .run(provider.cli(), &args)
            .and_then(|json| match provider {
                Provider::Claude => parse_claude_available(&json),
                Provider::Codex => parse_codex_available(&json),
            });
        match listed {
            Ok(listed) => {
                for (offer, installed) in listed {
                    let plugin = plugins
                        .entry(offer.id.clone())
                        .or_insert_with(|| offer.clone());
                    if plugin.description.is_empty() {
                        plugin.description = offer.description;
                    }
                    plugin.install_count = plugin.install_count.or(offer.install_count);
                    plugin.providers.insert(provider, installed);
                }
            }
            Err(e) => errors.push(format!("{}: {e:#}", provider.label())),
        }
    }
    let mut plugins: Vec<_> = plugins.into_values().collect();
    plugins.sort_by(|a, b| {
        b.install_count
            .cmp(&a.install_count)
            .then_with(|| a.id.cmp(&b.id))
    });
    (plugins, errors)
}

/// `claude plugin list --available --json`.
fn parse_claude_available(json: &str) -> Result<Vec<(AvailablePlugin, bool)>> {
    #[derive(Deserialize)]
    struct Installed {
        id: String,
        scope: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Offer {
        plugin_id: String,
        name: String,
        marketplace_name: String,
        #[serde(default)]
        description: String,
        install_count: Option<u64>,
    }
    #[derive(Deserialize)]
    struct List {
        installed: Vec<Installed>,
        available: Vec<Offer>,
    }
    let list: List =
        serde_json::from_str(json).context("unexpected `claude plugin list` output")?;
    let installed: Vec<String> = list
        .installed
        .into_iter()
        .filter(|i| i.scope == "user")
        .map(|i| i.id)
        .collect();
    Ok(list
        .available
        .into_iter()
        .map(|o| {
            let is_installed = installed.contains(&o.plugin_id);
            let offer = AvailablePlugin {
                id: o.plugin_id,
                name: o.name,
                marketplace: o.marketplace_name,
                description: o.description,
                install_count: o.install_count,
                providers: BTreeMap::new(),
            };
            (offer, is_installed)
        })
        .collect())
}

/// `codex plugin list --available --json`. Descriptions come from the manifests of plugins
/// whose marketplace is checked out locally.
fn parse_codex_available(json: &str) -> Result<Vec<(AvailablePlugin, bool)>> {
    #[derive(Deserialize)]
    struct Source {
        path: Option<PathBuf>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Offer {
        plugin_id: String,
        name: String,
        marketplace_name: String,
        installed: bool,
        source: Source,
    }
    #[derive(Deserialize)]
    struct List {
        available: Vec<Offer>,
    }
    let list: List = serde_json::from_str(json).context("unexpected `codex plugin list` output")?;
    Ok(list
        .available
        .into_iter()
        .map(|o| {
            let offer = AvailablePlugin {
                id: o.plugin_id,
                name: o.name,
                marketplace: o.marketplace_name,
                description: o
                    .source
                    .path
                    .as_deref()
                    .map(description)
                    .unwrap_or_default(),
                install_count: None,
                providers: BTreeMap::new(),
            };
            (offer, o.installed)
        })
        .collect())
}

/// `description` from the plugin manifest, whichever provider's manifest the plugin ships.
fn description(dir: &Path) -> String {
    [".claude-plugin", ".codex-plugin"]
        .iter()
        .filter_map(|m| fs::read_to_string(dir.join(m).join("plugin.json")).ok())
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .find_map(|v| v["description"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_available_lists() {
        let claude = r#"{"installed":[{"id":"a@m","scope":"user"},{"id":"b@m","scope":"project"}],
            "available":[{"pluginId":"a@m","name":"a","marketplaceName":"m","description":"A.","installCount":5},
                         {"pluginId":"b@m","name":"b","marketplaceName":"m"}]}"#;
        let listed = parse_claude_available(claude).unwrap();
        assert_eq!((listed[0].0.install_count, listed[0].1), (Some(5), true));
        assert_eq!((listed[1].0.description.as_str(), listed[1].1), ("", false));
        let codex = r#"{"installed":[],"available":[{"pluginId":"r@n","name":"r","marketplaceName":"n",
            "installed":true,"enabled":true,"source":{"source":"remote","id":"x"}}]}"#;
        let listed = parse_codex_available(codex).unwrap();
        assert_eq!((listed[0].0.id.as_str(), listed[0].1), ("r@n", true));
    }

    #[test]
    fn parses_claude_list_and_skips_project_scope() {
        let json = r#"[
            {"id":"a@m","version":"1.0","scope":"user","enabled":true,"installPath":"/x/a","notes":[]},
            {"id":"b@m","version":"2.0","scope":"project","enabled":true,"installPath":"/x/b"}
        ]"#;
        assert_eq!(
            parse_claude(json).unwrap(),
            [(
                "a@m".to_owned(),
                PluginInstall {
                    version: "1.0".into(),
                    path: Some("/x/a".into()),
                    enabled: true,
                    default: false
                }
            )]
        );
    }

    #[test]
    fn parses_codex_list_and_finds_cache_dir() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let dir = env.codex_plugin_cache().join("m/a/1.2");
        fs::create_dir_all(dir.join(".codex-plugin")).unwrap();
        fs::write(
            dir.join(".codex-plugin/plugin.json"),
            r#"{"description":"Does a."}"#,
        )
        .unwrap();
        let json = r#"{"installed":[
            {"pluginId":"a@m","name":"a","marketplaceName":"m","version":"1.2","installed":true,"enabled":false},
            {"pluginId":"r@m","name":"r","marketplaceName":"m","version":"0.1","installed":true,"enabled":true,"installPolicy":"INSTALLED_BY_DEFAULT"}
        ],"available":[]}"#;
        let listed = parse_codex(&env, json).unwrap();
        assert_eq!(
            listed[0].1,
            PluginInstall {
                version: "1.2".into(),
                path: Some(dir.clone()),
                enabled: false,
                default: false
            }
        );
        assert_eq!(listed[1].1.path, None);
        assert!(listed[1].1.default);
        assert_eq!(description(&dir), "Does a.");
    }
}
