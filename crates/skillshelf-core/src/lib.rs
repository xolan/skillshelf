//! Inventory and operations for agent skills and plugins.

mod config;
mod discover;
mod env;
mod ops;
mod plugins;
mod skills;

pub use discover::{
    FetchedSkill, SkillHit, SkillUpdate, check_skill_update, clean_staging, fetch_skill,
    fetch_update, gh_token, github_rate_limit, install_skill, search_skills, update_skill,
};
pub use env::Env;
pub use ops::{
    Action, execute, install_plugin, toggle_plugin, toggle_skill, uninstall_plugin, uninstall_skill,
};
pub use plugins::{AvailablePlugin, Plugin, PluginInstall, available_plugins, scan_plugins};
pub use skills::{Skill, SkillInstall, scan_skills};

#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    /// Links the folder `link` to `target`, which may be relative to the link's parent, the way
    /// `npx skills` links skills: a symlink, or a junction with an absolute target on Windows.
    pub fn link_dir(target: impl AsRef<Path>, link: impl AsRef<Path>) {
        let link = link.as_ref();
        #[cfg(windows)]
        let target = link.parent().unwrap().join(target);
        crate::ops::link_dir(target.as_ref(), link).unwrap();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    /// Name of the provider's command-line tool.
    pub fn cli(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}
