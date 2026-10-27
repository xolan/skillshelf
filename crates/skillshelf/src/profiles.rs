//! Profiles, saved in `settings.json`: the default one, which can't be renamed or removed, then
//! the ones the user adds. The active profile decides which agents skillshelf manages, and keeps
//! how their skills and plugins are set; switching to another applies what that one kept.

use anyhow::{Result, bail, ensure};
use serde_json::Value;
use skillshelf_core::{Profile, Provider};

use crate::settings;

pub const DEFAULT: &str = "Default";

#[derive(Clone, Debug, PartialEq)]
pub struct Profiles {
    /// The default profile first, then the others in the order they were added.
    pub list: Vec<Profile>,
    /// Index of the profile in use.
    pub active: usize,
}

impl Profiles {
    pub fn load() -> Profiles {
        Profiles::from_settings(&settings::load())
    }

    fn from_settings(saved: &Value) -> Profiles {
        let mut list: Vec<Profile> = saved["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| serde_json::from_value::<Profile>(p.clone()).ok())
            .filter(|p| !p.name.trim().is_empty())
            .collect();
        match list.iter().position(|p| p.name == DEFAULT) {
            Some(ix) => {
                let default = list.remove(ix);
                list.insert(0, default);
            }
            // Before profiles, the agents were a setting of their own.
            None => list.insert(
                0,
                Profile::new(DEFAULT, settings::agents_in(&saved["agents"])),
            ),
        }
        for profile in &mut list {
            profile.agents = in_display_order(&profile.agents);
        }
        let active = saved["profile"]
            .as_str()
            .and_then(|name| list.iter().position(|p| p.name == name))
            .unwrap_or(0);
        Profiles { list, active }
    }

    pub fn save(&self) -> Result<()> {
        let list = serde_json::to_value(&self.list)?;
        settings::update(|saved| {
            saved["profiles"] = list;
            saved["profile"] = self.active().name.clone().into();
            // The default profile holds them now.
            if let Some(saved) = saved.as_object_mut() {
                saved.remove("agents");
            }
        })
    }

    pub fn active(&self) -> &Profile {
        &self.list[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Profile {
        &mut self.list[self.active]
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|p| p.name == name)
    }

    /// Every agent some profile manages, in display order.
    pub fn all_agents(&self) -> Vec<Provider> {
        let agents: Vec<Provider> = self.list.iter().flat_map(|p| p.agents.clone()).collect();
        in_display_order(&agents)
    }

    /// The trimmed name, if no other profile than the one at `renaming` has it.
    pub fn check_name(&self, name: &str, renaming: Option<usize>) -> Result<String> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Give the profile a name.");
        ensure!(
            name.chars().count() <= MAX_NAME,
            "Keep the name to {MAX_NAME} characters."
        );
        let taken = self
            .list
            .iter()
            .position(|p| p.name.to_lowercase() == name.to_lowercase());
        if let Some(ix) = taken
            && Some(ix) != renaming
        {
            bail!("There is a profile named “{}” already.", self.list[ix].name);
        }
        Ok(name.to_owned())
    }

    /// Adds a copy of the active profile and returns where it is.
    pub fn add(&mut self, name: &str) -> Result<usize> {
        let name = self.check_name(name, None)?;
        let profile = Profile {
            name,
            ..self.active().clone()
        };
        self.list.push(profile);
        Ok(self.list.len() - 1)
    }

    pub fn rename(&mut self, ix: usize, name: &str) -> Result<()> {
        ensure!(ix != 0, "The default profile can't be renamed.");
        self.list[ix].name = self.check_name(name, Some(ix))?;
        Ok(())
    }

    pub fn remove(&mut self, ix: usize) -> Result<()> {
        ensure!(ix != 0, "The default profile can't be removed.");
        ensure!(
            ix != self.active,
            "Switch to another profile before removing this one."
        );
        self.list.remove(ix);
        if ix < self.active {
            self.active -= 1;
        }
        Ok(())
    }
}

const MAX_NAME: usize = 40;

fn in_display_order(agents: &[Provider]) -> Vec<Provider> {
    Provider::ALL
        .into_iter()
        .filter(|p| agents.contains(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skillshelf_core::Provider;

    use super::{DEFAULT, Profiles};

    #[test]
    fn the_default_profile_takes_over_the_agents_setting() {
        let profiles = Profiles::from_settings(&json!({"agents": ["codex"], "github": "gh"}));
        assert_eq!(profiles.list.len(), 1);
        assert_eq!(profiles.active().name, DEFAULT);
        assert_eq!(profiles.active().agents, [Provider::Codex]);
        assert_eq!(Profiles::from_settings(&json!({})).active().agents, []);
    }

    #[test]
    fn loads_the_default_first_and_the_saved_active_one() {
        let profiles = Profiles::from_settings(&json!({
            "agents": ["claude"],
            "profile": "Work",
            "profiles": [
                {"name": "Work", "agents": ["codex", "claude"]},
                {"name": "Default", "agents": []},
                {"name": "", "agents": []},
                "not a profile"
            ]
        }));
        let names: Vec<&str> = profiles.list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, [DEFAULT, "Work"]);
        assert_eq!(profiles.active, 1);
        assert_eq!(profiles.active().agents, Provider::ALL);
        assert_eq!(profiles.list[0].agents, []);
        assert_eq!(profiles.all_agents(), Provider::ALL);
    }

    #[test]
    fn adds_renames_and_removes() {
        let mut profiles = Profiles::from_settings(&json!({"agents": ["claude"]}));
        let work = profiles.add("  Work ").unwrap();
        assert_eq!(profiles.list[work].name, "Work");
        assert_eq!(profiles.list[work].agents, [Provider::Claude]);
        assert!(profiles.add("work").is_err());
        assert!(profiles.add(" ").is_err());
        assert!(profiles.add("default").is_err());
        assert!(profiles.rename(0, "Mine").is_err());
        profiles.rename(work, "WORK").unwrap();
        assert_eq!(profiles.list[work].name, "WORK");

        let home = profiles.add("Home").unwrap();
        profiles.active = home;
        assert!(profiles.remove(0).is_err());
        assert!(profiles.remove(home).is_err());
        profiles.remove(work).unwrap();
        assert_eq!(profiles.active().name, "Home");
    }
}
