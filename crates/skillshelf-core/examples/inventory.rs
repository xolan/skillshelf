//! Prints the installed skills and plugins. Read-only.

use skillshelf_core::{Env, Provider, scan_plugins, scan_skills};

fn main() -> anyhow::Result<()> {
    let env = Env::detect()?;
    let state = |on: Option<bool>| match on {
        None => "-",
        Some(true) => "on",
        Some(false) => "off",
    };
    for skill in scan_skills(&env)? {
        println!(
            "skill  {:<45} claude={:<3} codex={:<3} {}",
            skill.name,
            state(skill.enabled(Provider::Claude)),
            state(skill.enabled(Provider::Codex)),
            skill.source.as_deref().unwrap_or(""),
        );
    }
    let (plugins, errors) = scan_plugins(&env);
    for plugin in plugins {
        let on = |p| state(plugin.installs.get(&p).map(|i| i.enabled));
        println!(
            "plugin {:<45} claude={:<3} codex={:<3}",
            plugin.id,
            on(Provider::Claude),
            on(Provider::Codex)
        );
    }
    for error in errors {
        eprintln!("error: {error}");
    }
    Ok(())
}
