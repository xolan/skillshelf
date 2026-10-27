//! Profiles: named sets of the agents skillshelf manages and the skills and plugins each has on.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Action, Env, Plugin, Provider, Skill, toggle_plugin, toggle_skill};

/// On or off, per agent.
pub type States = BTreeMap<Provider, bool>;

/// The agents skillshelf manages, and which skills and plugins are on for each of them. Applying
/// a profile changes only what it mentions: skills and plugins it has never seen stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// In display order.
    #[serde(default)]
    pub agents: Vec<Provider>,
    /// By skill directory name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skills: BTreeMap<String, States>,
    /// By plugin id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, States>,
}

impl Profile {
    pub fn new(name: impl Into<String>, agents: Vec<Provider>) -> Profile {
        Profile {
            name: name.into(),
            agents,
            ..Profile::default()
        }
    }

    /// Whether the profile has the skill on for `agent`: as saved, or else as it is now. `None`
    /// when the agent can't have it.
    pub fn skill_on(&self, skill: &Skill, agent: Provider) -> Option<bool> {
        let now = skill_state(skill, agent)?;
        Some(saved(&self.skills, &skill.name, agent).unwrap_or(now))
    }

    /// Whether the profile has the plugin on for `agent`: as saved, or else as it is now. `None`
    /// when the agent doesn't have it installed, or installed it by default.
    pub fn plugin_on(&self, plugin: &Plugin, agent: Provider) -> Option<bool> {
        let now = plugin_state(plugin, agent)?;
        Some(saved(&self.plugins, &plugin.id, agent).unwrap_or(now))
    }

    /// Turns managing `agent` on or off, keeping the agents in display order.
    pub fn set_agent(&mut self, agent: Provider, on: bool) {
        self.agents = Provider::ALL
            .into_iter()
            .filter(|&p| {
                if p == agent {
                    on
                } else {
                    self.agents.contains(&p)
                }
            })
            .collect();
    }

    pub fn set_skill(&mut self, name: &str, agent: Provider, on: bool) {
        self.skills
            .entry(name.to_owned())
            .or_default()
            .insert(agent, on);
    }

    pub fn set_plugin(&mut self, id: &str, agent: Provider, on: bool) {
        self.plugins
            .entry(id.to_owned())
            .or_default()
            .insert(agent, on);
    }

    /// Saves how the profile's agents have skills and plugins set now, and returns whether that
    /// changed anything. A skill an agent can no longer have is forgotten for it; a plugin missing
    /// from the list keeps what was saved, so a provider whose tool failed loses nothing.
    pub fn record(&mut self, skills: &[Skill], plugins: &[Plugin]) -> bool {
        let before = self.clone();
        let agents = self.agents.clone();
        for skill in skills {
            for &agent in &agents {
                match skill_state(skill, agent) {
                    Some(on) => self.set_skill(&skill.name, agent, on),
                    None => {
                        if let Some(states) = self.skills.get_mut(&skill.name) {
                            states.remove(&agent);
                            if states.is_empty() {
                                self.skills.remove(&skill.name);
                            }
                        }
                    }
                }
            }
        }
        for plugin in plugins {
            for &agent in &agents {
                if let Some(on) = plugin_state(plugin, agent) {
                    self.set_plugin(&plugin.id, agent, on);
                }
            }
        }
        *self != before
    }

    /// Steps that turn the profile's agents' skills and plugins on and off as it has them.
    pub fn apply(&self, env: &Env, skills: &[Skill], plugins: &[Plugin]) -> Vec<Action> {
        let mut steps = Vec::new();
        for skill in skills {
            for &agent in &self.agents {
                if let (Some(want), Some(now)) = (
                    saved(&self.skills, &skill.name, agent),
                    skill_state(skill, agent),
                ) && want != now
                {
                    steps.extend(toggle_skill(env, skill, agent, want));
                }
            }
        }
        for plugin in plugins {
            for &agent in &self.agents {
                if let (Some(want), Some(now)) = (
                    saved(&self.plugins, &plugin.id, agent),
                    plugin_state(plugin, agent),
                ) && want != now
                {
                    steps.extend(toggle_plugin(plugin, agent, want));
                }
            }
        }
        steps
    }
}

/// Whether `agent` has the skill on, or `None` when it can't turn it on or off: it doesn't have
/// it, or the skill ships with the agent. A shared skill that is not linked into Claude is off
/// there, because turning it on links it in.
pub fn skill_state(skill: &Skill, agent: Provider) -> Option<bool> {
    if skill.default {
        return None;
    }
    skill
        .enabled(agent)
        .or((agent == Provider::Claude && skill.shared.is_some()).then_some(false))
}

/// Whether `agent` has the plugin on, or `None` when it can't turn it on or off: it doesn't have
/// it installed, or installed it by default.
fn plugin_state(plugin: &Plugin, agent: Provider) -> Option<bool> {
    let install = plugin.installs.get(&agent)?;
    (!install.default).then_some(install.enabled)
}

fn saved(items: &BTreeMap<String, States>, key: &str, agent: Provider) -> Option<bool> {
    items.get(key)?.get(&agent).copied()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::{PluginInstall, execute, scan_skills};

    fn write_skill(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: s\ndescription: d\n---\n").unwrap();
    }

    fn plugin(installs: &[(Provider, bool)]) -> Plugin {
        Plugin {
            id: "p@m".into(),
            name: "p".into(),
            marketplace: "m".into(),
            description: String::new(),
            installs: installs
                .iter()
                .map(|&(provider, enabled)| {
                    let install = PluginInstall {
                        version: "1.0".into(),
                        path: None,
                        enabled,
                        default: false,
                    };
                    (provider, install)
                })
                .collect(),
        }
    }

    #[test]
    fn applying_a_recorded_profile_restores_it() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        write_skill(&env.agents_skills().join("s"));
        let mut work = Profile::new("Work", Provider::ALL.to_vec());
        // Not linked into Claude yet: off there, on for Codex.
        assert!(work.record(&scan_skills(&env).unwrap(), &[]));
        assert_eq!(
            work.skills["s"],
            States::from([(Provider::Claude, false), (Provider::Codex, true)])
        );
        assert!(!work.record(&scan_skills(&env).unwrap(), &[]));

        let mut home_profile = work.clone();
        home_profile.set_skill("s", Provider::Claude, true);
        home_profile.set_skill("s", Provider::Codex, false);
        let steps = home_profile.apply(&env, &scan_skills(&env).unwrap(), &[]);
        assert_eq!(steps.len(), 3, "{steps:?}");
        execute(&env, &steps).unwrap();
        let skills = scan_skills(&env).unwrap();
        assert_eq!(skills[0].enabled(Provider::Claude), Some(true));
        assert_eq!(skills[0].enabled(Provider::Codex), Some(false));
        assert!(home_profile.apply(&env, &skills, &[]).is_empty());

        execute(&env, &work.apply(&env, &skills, &[])).unwrap();
        let skills = scan_skills(&env).unwrap();
        assert_eq!(skills[0].enabled(Provider::Claude), Some(false));
        assert_eq!(skills[0].enabled(Provider::Codex), Some(true));
    }

    #[test]
    fn apply_leaves_other_agents_and_unknown_items_alone() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        write_skill(&env.agents_skills().join("s"));
        let skills = scan_skills(&env).unwrap();
        let installed = [plugin(&[(Provider::Claude, true), (Provider::Codex, true)])];

        let mut profile = Profile::new("Claude only", vec![Provider::Claude]);
        assert!(profile.apply(&env, &skills, &installed).is_empty());
        profile.set_skill("s", Provider::Codex, false);
        profile.set_plugin("p@m", Provider::Codex, false);
        assert!(profile.apply(&env, &skills, &installed).is_empty());

        profile.set_plugin("p@m", Provider::Claude, false);
        let steps = profile.apply(&env, &skills, &installed);
        assert_eq!(steps, toggle_plugin(&installed[0], Provider::Claude, false));
    }

    #[test]
    fn record_forgets_what_an_agent_lost_but_keeps_unlisted_plugins() {
        let mut profile = Profile::new("Default", vec![Provider::Codex]);
        profile.set_skill("gone", Provider::Codex, true);
        profile.set_plugin("p@m", Provider::Codex, false);
        let skill = Skill {
            name: "gone".into(),
            title: "gone".into(),
            description: String::new(),
            shared: None,
            source: None,
            installs: Vec::new(),
            default: false,
        };
        // Codex's tool failed, so the plugin was listed for Claude only.
        assert!(profile.record(&[skill], &[plugin(&[(Provider::Claude, true)])]));
        assert!(profile.skills.is_empty());
        assert_eq!(
            profile.plugins["p@m"],
            States::from([(Provider::Codex, false)])
        );
    }

    #[test]
    fn shows_saved_choices_over_the_current_state() {
        let mut profile = Profile::new("Default", vec![Provider::Claude]);
        let installed = plugin(&[(Provider::Claude, true)]);
        assert_eq!(profile.plugin_on(&installed, Provider::Claude), Some(true));
        assert_eq!(profile.plugin_on(&installed, Provider::Codex), None);
        profile.set_plugin("p@m", Provider::Claude, false);
        assert_eq!(profile.plugin_on(&installed, Provider::Claude), Some(false));

        // A default install can't be turned off, so the profile leaves it alone.
        let mut default = installed.clone();
        default.installs.get_mut(&Provider::Claude).unwrap().default = true;
        assert_eq!(profile.plugin_on(&default, Provider::Claude), None);
        assert!(!profile.record(&[], std::slice::from_ref(&default)));
        let env = Env::at(Path::new("/h"));
        assert!(profile.apply(&env, &[], &[default]).is_empty());
    }

    #[test]
    fn serializes_providers_by_their_tool_names() {
        let mut profile = Profile::new("Work", vec![Provider::Claude]);
        profile.set_plugin("p@m", Provider::Codex, true);
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(
            json,
            r#"{"name":"Work","agents":["claude"],"plugins":{"p@m":{"codex":true}}}"#
        );
        assert_eq!(serde_json::from_str::<Profile>(&json).unwrap(), profile);
    }
}
