# skillshelf

A desktop app for managing agent skills and plugins for Claude Code and Codex CLI.

- Browse, enable, disable and uninstall installed skills and plugins, per provider or for all at once.
- Search [skills.sh](https://skills.sh) for skills and provider marketplaces for plugins, and install them.
- Skills live in `~/.agents/skills/` and are shared with `npx skills` through `~/.agents/.skill-lock.json`.
- Skills installed from GitHub show when a newer version is out, and update in place.

Linux only for now. See [PLAN.md](PLAN.md).

## Development

Tooling is pinned in `mise.toml`; run every developer command through [mise](https://mise.jdx.dev).
Linux builds also need these system libraries:

```bash
sudo apt-get install -y libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev
```

| Command | What it does |
| --- | --- |
| `mise run run` | Run the app |
| `mise run ci` | Formatting, clippy, workflow lint and tests, as CI runs them |
| `mise run fmt` | Format the code |
| `mise run build` | Build a release binary |
| `mise run package <target>` | Build a release tarball into `dist/` |
| `mise run inventory` | Print installed skills and plugins (read-only) |

## Keyboard

| Key | What it does |
| --- | --- |
| `Ctrl+K` | Command palette: go to a page, reload, pick a theme |
| `Up` / `Down` | Move through the list, also from the filter field |
| `Enter` | Install the selected search result, or update the selected skill |
| `Ctrl+F` | Focus the filter |
| `Ctrl+R` | Reload |
| `Ctrl+Q` | Quit |

## Desktop entry

The release tarball holds `skillshelf.desktop`. To add skillshelf to your app menu:

```bash
install -Dm644 skillshelf.desktop ~/.local/share/applications/skillshelf.desktop
```

Put the `skillshelf` binary on your `PATH` (e.g. `~/.local/bin`).

## Themes

skillshelf ships its own light and dark themes and a set of Base16 schemes. Drop more Base16
scheme files (`*.yaml`) into `~/.config/skillshelf/themes/` and pick them from the palette button
in the title bar.

The UI font is [Geist](https://github.com/vercel/geist-font), embedded under the SIL Open Font
License (`OFL-Geist.txt` in the release tarball).

## Releases

Pushing to `main` with a new `version` in `Cargo.toml` builds Linux binaries and publishes a GitHub release `v<version>`.
