<p align="center"><img src="assets/brand/skillshelf-wordmark.svg" alt="skillshelf" width="236"></p>

# skillshelf

A desktop app for managing agent skills and plugins for Claude Code and Codex CLI.

- Browse, enable, disable and uninstall installed skills and plugins, per provider or for all at once.
- Search [skills.sh](https://skills.sh) for skills and provider marketplaces for plugins, and install them.
- Skills live in `~/.agents/skills/` and are shared with `npx skills` through `~/.agents/.skill-lock.json`.
- Skills installed from GitHub show when a newer version is out, and update in place.
- Pick the agents it manages in Settings; it leaves the others alone. None are on until you turn
  them on.
- Keep profiles: each one holds the agents it manages and which of their skills and plugins are
  on. Switch from the title bar to turn everything on and off as that profile has it. The Default
  profile can't be removed; the profile in use keeps whatever you change while it is active.

Runs on Linux and Windows.

## Install

### Linux

Download a tarball from [Releases](https://github.com/xolan/skillshelf/releases), then:

```bash
tar -xzf skillshelf-*-x86_64-unknown-linux-gnu.tar.gz
cd skillshelf-*/
install -Dm755 skillshelf ~/.local/bin/skillshelf
install -Dm644 skillshelf.desktop ~/.local/share/applications/skillshelf.desktop
install -Dm644 skillshelf.svg ~/.local/share/icons/hicolor/scalable/apps/skillshelf.svg
```

### Windows

Download `skillshelf-<version>-x86_64-pc-windows-msvc.zip` from
[Releases](https://github.com/xolan/skillshelf/releases) (`aarch64` for Arm PCs), extract it
somewhere it can stay, such as `%LOCALAPPDATA%\Programs\skillshelf`, and run `skillshelf.exe`. To
start it from the Start menu, right-click `skillshelf.exe` and choose Pin to Start.

The executable isn't signed yet, so on first launch SmartScreen says "Windows protected your PC":
choose More info, then Run anyway.

skillshelf runs `claude`, `codex` and `gh` from your `PATH`, including the `.cmd` files npm
installs, and links skills into Claude Code with directory junctions, as `npx skills` does.

## Development

Tooling is pinned in `mise.toml`; run every developer command through [mise](https://mise.jdx.dev).
Linux builds also need these system libraries:

```bash
sudo apt-get install -y libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev
```

Windows builds need the Visual Studio Build Tools with the "Desktop development with C++" workload
and a Windows SDK (gpui compiles its shaders with the SDK's `fxc.exe`), Git for Windows (mise runs
the bash tasks with its bash), and Python 3 for the licenses tasks.

| Command | What it does |
| --- | --- |
| `mise run run` | Run the app |
| `mise run ci` | Formatting, clippy, workflow lint and tests, as CI runs them |
| `mise run fmt` | Format the code |
| `mise run build` | Build a release binary |
| `mise run package <target>` | Build a release archive into `dist/`: a tarball, or a zip for Windows |
| `mise run inventory` | Print installed skills and plugins (read-only) |
| `mise run licenses` | Regenerate `THIRD_PARTY_LICENSES.md` and the app's `licenses.json` after changing dependencies |
| `mise run icon` | Render the Windows app icon from `assets/brand/skillshelf.svg` |

## Keyboard

| Key | What it does |
| --- | --- |
| `Ctrl+K` | Command palette: go to a page, reload, pick a theme |
| `Up` / `Down` | Move through the list, also from the filter field |
| `Enter` | Install the selected search result, or update the selected skill |
| `Ctrl+F` | Focus the filter |
| `Ctrl+R` | Reload |
| `Ctrl+Q` | Quit |

## Desktop entry (Linux)

The release tarball holds `skillshelf.desktop`. To add skillshelf to your app menu:

```bash
install -Dm644 skillshelf.desktop ~/.local/share/applications/skillshelf.desktop
```

Put the `skillshelf` binary on your `PATH` (e.g. `~/.local/bin`).

## Themes

skillshelf ships its own light and dark themes and a set of Base16 schemes. Drop more Base16
scheme files (`*.yaml`) into `~/.config/skillshelf/themes/` (`%APPDATA%\skillshelf\themes\` on
Windows) and pick them under Settings → Theme, or from the command palette (`Ctrl+K`).

The UI font is [Geist](https://github.com/vercel/geist-font), embedded under the SIL Open Font
License (`OFL-Geist.txt` in the release archive).

## Releases

Pushing to `main` with a new `version` in `Cargo.toml` builds Linux and Windows binaries (x86_64 and
aarch64) and publishes a GitHub release `v<version>`.

## License

[MIT](LICENSE). Third-party crates, fonts, icons and color schemes are credited in
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md), also shown in the app under Settings → Third-party licenses.
