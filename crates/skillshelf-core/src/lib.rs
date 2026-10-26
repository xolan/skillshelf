//! Inventory and operations for agent skills and plugins.

mod config;
mod env;
mod ops;
mod plugins;
mod skills;

pub use env::Env;
pub use ops::{Action, execute, toggle_plugin, toggle_skill, uninstall_plugin, uninstall_skill};
pub use plugins::{Plugin, PluginInstall, scan_plugins};
pub use skills::{Skill, SkillInstall, scan_skills};

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
