<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/xolkit-wordmark.svg">
    <img src="assets/brand/xolkit-wordmark-light.svg" alt="xolkit" width="210">
  </picture>
</p>

# xolkit

A desktop app for managing agent skills and plugins for Claude Code and Codex CLI. It was called
skillshelf before 2.0; see [Upgrading from skillshelf](#upgrading-from-skillshelf).

- Browse, enable, disable and uninstall installed skills and plugins, per provider or for all at once.
- Search [skills.sh](https://skills.sh) for skills and provider marketplaces for plugins, and install them.
- Skills live in `~/.agents/skills/` and are shared with `npx skills` through `~/.agents/.skill-lock.json`.
- Skills installed from GitHub show when a newer version is out, and update in place.
- Pick the agents it manages in Settings; it leaves the others alone. None are on until you turn
  them on.
- Keep profiles: each one holds the agents it manages and which of their skills and plugins are
  on. Switch from the title bar to turn everything on and off as that profile has it. The Default
  profile can't be removed; the profile in use keeps whatever you change while it is active.

Runs on Linux and Windows. There is no macOS build.

## Requirements

- Linux: x86_64 or aarch64 with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and
  later), a Wayland or X11 session, and working GPU drivers.
- Windows: Windows 10 or 11, x64 or Arm64.
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`), [Codex
  CLI](https://github.com/openai/codex) (`codex`), or both, on your `PATH`. xolkit runs them to
  install and remove plugins. Turning skills on and off edits their config files directly.
- Optional: the [GitHub CLI](https://cli.github.com) (`gh`), signed in with `gh auth login`.
  xolkit asks it for a token when it downloads skills from GitHub. Without it, pick anonymous
  access (60 requests an hour) or paste a token under Settings → GitHub access.

xolkit talks to `skills.sh`, `api.github.com`, `codeload.github.com` and
`raw.githubusercontent.com`.

## Install

Each release holds a Linux tarball and a Windows zip for x86_64 and aarch64, and a `PKGBUILD`
for Arch Linux, each with a `.sha256` checksum file.

### Linux

Download `xolkit-<version>-<arch>-unknown-linux-gnu.tar.gz` and its `.sha256` from
[Releases](https://github.com/xolan/skillshelf/releases), where `<arch>` is `x86_64` or `aarch64`
(`uname -m` tells you which). In the folder you downloaded them to, set `version` to the release
you picked and run:

```bash
version=2.0.0
name=xolkit-$version-$(uname -m)-unknown-linux-gnu
sha256sum -c "$name.tar.gz.sha256" &&
  tar -xzf "$name.tar.gz" &&
  install -Dm755 "$name/xolkit" ~/.local/bin/xolkit &&
  install -Dm644 "$name/xolkit.desktop" ~/.local/share/applications/xolkit.desktop &&
  cp -r "$name/icons/." ~/.local/share/icons/
```

If the checksum doesn't match, nothing is installed.

The desktop entry starts `xolkit` from your `PATH`. If it doesn't show in your app menu, or
won't start from there, check that `~/.local/bin` is on the `PATH` of your desktop session, not
only your shell's, and log out and back in.

### Arch Linux

On Arch and other pacman-based distributions, download `PKGBUILD` and `PKGBUILD.sha256` from
[Releases](https://github.com/xolan/skillshelf/releases) into an empty folder, and in that folder
run:

```bash
sha256sum -c PKGBUILD.sha256 && makepkg -si
```

This builds a `xolkit-bin` package from the release's prebuilt binary, checks the tarball
against the checksum in the `PKGBUILD`, and installs it with pacman, which pulls in the libraries it
needs. If you have no Vulkan driver yet, pacman asks you to pick the one for your GPU, such as
`vulkan-radeon`, `vulkan-intel` or `nvidia-utils`.

If you installed from the tarball before, remove that copy first (the first `rm` command under
[Uninstall](#uninstall)), or the one in `~/.local/bin` keeps starting instead.

### Windows

Download `xolkit-<version>-x86_64-pc-windows-msvc.zip` from
[Releases](https://github.com/xolan/skillshelf/releases) (`aarch64` for Arm PCs), extract it
somewhere it can stay, such as `%LOCALAPPDATA%\Programs\xolkit`, and run `xolkit.exe`. To
start it from the Start menu, right-click `xolkit.exe` and choose Pin to Start.

The executable isn't signed yet, so on first launch SmartScreen says "Windows protected your PC":
choose More info, then Run anyway.

xolkit finds `claude`, `codex` and `gh` on your `PATH`, including the `.cmd` files npm
installs, and links skills into Claude Code with directory junctions, as `npx skills` does.

### Upgrading from skillshelf

xolkit moves skillshelf's settings, profiles and saved token to its own folder the first time it
starts (see [Where it keeps its files](#where-it-keeps-its-files)). Remove the old app yourself:
`sudo pacman -R skillshelf-bin` for the Arch package (installing `xolkit-bin` offers to do it),
or for the tarball:

```bash
rm ~/.local/bin/skillshelf \
  ~/.local/share/applications/skillshelf.desktop \
  ~/.local/share/icons/hicolor/scalable/apps/skillshelf.svg
```

On Windows, delete the skillshelf folder and unpin it from Start.

### First run

Open Settings and turn on the agents you want xolkit to manage. Until you do, it leaves
their skills, plugins and config files alone.

## Where it keeps its files

xolkit's own files are in `~/.config/xolkit/` (`$XDG_CONFIG_HOME/xolkit/` if you set
it), or `%APPDATA%\xolkit\` on Windows:

| File | What it holds |
| --- | --- |
| `settings.json` | Theme, managed agents, GitHub access choice and profiles |
| `github-token` | The token you saved under Settings, in plain text. On Linux the file has mode `0600`, so only you and root can read it; on Windows it gets the permissions of your `%APPDATA%` folder. |
| `themes/` | Extra Base16 themes you add |

It also changes files that belong to the agents and to `npx skills`:

- `~/.agents/skills/` and `~/.agents/.skill-lock.json`, for skills it installs, updates or removes.
- `~/.claude/settings.json` and `~/.codex/config.toml`, to turn skills and plugins on and off.
- `~/.claude/skills/`, where it links shared skills for Claude Code (Codex reads them from
  `~/.agents/skills/` directly). It also removes skills you uninstall from here and from
  `~/.codex/skills/`.
- Claude Code's plugin folders under `~/.claude/plugins/`, which it removes after
  `claude plugin uninstall`.

`$CLAUDE_CONFIG_DIR` and `$CODEX_HOME` move `~/.claude` and `~/.codex`, as they do for the agents.

Downloads wait in `~/.agents/.xolkit-staging/` until you install them, and xolkit clears
that folder each time it starts.

## Uninstall

On Linux, if you installed the Arch package, run `sudo pacman -R xolkit-bin` in place of the
first `rm` command, which removes the binary, desktop entry and icon:

```bash
rm ~/.local/bin/xolkit \
  ~/.local/share/applications/xolkit.desktop \
  ~/.local/share/icons/hicolor/*/apps/xolkit.svg
rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/xolkit"   # settings, profiles and saved token
rm -rf ~/.agents/.xolkit-staging
```

On Windows, delete the folder you extracted, unpin it from Start, and delete
`%APPDATA%\xolkit` and `%USERPROFILE%\.agents\.xolkit-staging`.

Skills and plugins you installed with xolkit stay installed, along with the agents' config
files. Remove them in xolkit first if you don't want them.

## Keyboard

| Key | What it does |
| --- | --- |
| `Ctrl+K` | Command palette: go to a page, reload, switch profile, pick a theme |
| `Up` / `Down` | Move through the list, also from the filter field |
| `Enter` | Install the selected search result, update the selected skill, or switch to the selected profile |
| `Delete` | Uninstall the selected skill or plugin, unless the filter field has focus |
| `Ctrl+F` | Focus the filter |
| `Ctrl+R` | Reload |
| `Ctrl+Q` | Quit |

## Themes

xolkit ships its own light and dark themes and a set of Base16 schemes. Drop more Base16
scheme files (`*.yaml`) into the `themes` folder in xolkit's config folder (see [Where it
keeps its files](#where-it-keeps-its-files)) and pick them under Settings → Theme, or from the
command palette (`Ctrl+K`).

The UI font is [Geist](https://github.com/vercel/geist-font), embedded under the SIL Open Font
License (`OFL-Geist.txt` in the release archive).

## Development

Tooling is pinned in `mise.toml`: Rust 1.99.1 with rustfmt and clippy, and actionlint. Run every
developer command through [mise](https://mise.jdx.dev); you don't need rustup.

You also need:

- Git and Python 3 (the licenses check in `mise run ci` uses it).
- On Linux, a C toolchain and these libraries. On Debian or Ubuntu:

  ```bash
  sudo apt-get install -y build-essential libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev
  ```

  On other distributions, install the development packages for fontconfig, wayland,
  libxkbcommon-x11 and libx11-xcb.
- On Windows, the Visual Studio Build Tools with the "Desktop development with C++" workload and a
  Windows SDK (gpui compiles its shaders with the SDK's `fxc.exe`), and Git for Windows (mise runs
  the bash tasks with its bash).

Then:

```bash
git clone https://github.com/xolan/skillshelf.git
cd skillshelf
mise trust
mise install
mise run run
```

| Command | What it does |
| --- | --- |
| `mise run run` | Run the app |
| `mise run test` | Run the tests |
| `mise run lint` | Check formatting, clippy and the GitHub workflows |
| `mise run ci` | Lint, tests and the licenses check, as CI runs them |
| `mise run fmt` | Format the code |
| `mise run build` | Build a release binary |
| `mise run package <target>` | Build a release archive into `dist/` for the host's own platform: a tarball on Linux, a zip on Windows |
| `mise run inventory` | Print installed skills and plugins (read-only) |
| `mise run licenses` | Regenerate `THIRD_PARTY_LICENSES.md` and the app's `licenses.json` after changing dependencies |
| `mise run icon` | Render the Windows app icon from the SVGs in `assets/brand/` |

A few tests call skills.sh and GitHub, so they are skipped by default. Run them with
`mise exec -- cargo test -p xolkit-core --locked -- --ignored`.

## Contributing

Issues and pull requests are welcome. Before you open a pull request, run `mise run ci`, and run
`mise run licenses` if you changed dependencies. Leave the version in `Cargo.toml` alone: a new
version on `main` publishes a release.

## Releases

Pushing to `main` with a new `version` in `Cargo.toml` runs CI, builds Linux and Windows binaries
(x86_64 and aarch64), writes the Arch Linux `PKGBUILD` from `assets/arch/PKGBUILD` with the
tarballs' checksums, and publishes a GitHub release `v<version>`.

## License

[MIT](LICENSE). Third-party crates, fonts, icons and color schemes are credited in
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md), also shown in the app under Settings → Third-party licenses.
