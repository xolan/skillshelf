# skillshelf

A desktop app for managing agent skills and plugins for Claude Code and Codex CLI.

- Browse, enable, disable and uninstall installed skills and plugins, per provider or for all at once.
- Search [skills.sh](https://skills.sh) for skills and provider marketplaces for plugins, and install them.
- Skills live in `~/.agents/skills/` and are shared with `npx skills` through `~/.agents/.skill-lock.json`.

Linux only for now. See [PLAN.md](PLAN.md).

## Build

```bash
sudo apt-get install -y libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev
cargo build --release
```

## Releases

Pushing to `main` with a new `version` in `Cargo.toml` builds Linux binaries and publishes a GitHub release `v<version>`.
