# Windows support handover

You are picking up skillshelf on a Windows machine. Your job is to **audit** what Windows needs, then implement it, so that skillshelf builds, runs, passes CI and ships a release on Windows (x86_64 first, then aarch64).

skillshelf is a Rust desktop app (GPUI via `gpui-kit`) that manages agent skills and plugins for Claude Code and Codex CLI. So far it has only been built and verified on Linux.

## Read first

- `PLAN.md`: decisions, provider facts (verified on Linux only), architecture, safety rules.
- `README.md` and `mise.toml`: how to build, test, package. Every developer command is a `mise run <task>`.
- `crates/skillshelf-core/` holds inventory, planning and execution (no UI). `crates/skillshelf/` is the GPUI app.

## How the user works

- **mise for every developer command.** When Windows needs a different command, add it to `mise.toml` (mise tasks support `run_windows`) instead of running it ad hoc.
- **Focus.** The user is usually working in other windows. Launch the GUI only after the user confirms it's a good moment. There is no headless display for GPUI on Windows, so prefer tests, and ask the user to try UI flows when needed.
- **Branch and PR.** Work on a branch, open a PR, and leave merging to the user. Keep the version in `Cargo.toml` unchanged, because a new version publishes a release.
- **Sandbox** every experiment that writes provider state. Point `CLAUDE_CONFIG_DIR` and `CODEX_HOME` at temporary folders, and set `XDG_CONFIG_HOME` (or its Windows replacement, see below) for skillshelf's own settings. Never install, toggle or uninstall anything in the user's real profile.
- **Real APIs only.** Check gpui-kit and gpui-component signatures in the registry sources (`%USERPROFILE%\.cargo\registry\src\*\gpui-component-0.8.1`) before using them.

## Steps

### 1. Toolchain

Install what the Windows build needs: mise, Git, `gh`, Python 3 (for `scripts/licenses.py`), Visual Studio Build Tools with the "Desktop development with C++" workload (MSVC linker), and the Windows SDK. The SDK matters because `gpui-pre-windows` compiles HLSL shaders with `fxc.exe` at build time and panics if it can't find it (set `GPUI_FXC_PATH` if it isn't on `PATH`).

Done when `mise install` succeeds and `mise run build` either succeeds or fails only on items in the audit checklist below.

### 2. Audit

Go through every item in the checklist below on the real machine. Mark each one **confirmed**, **differs** (with evidence such as a path or a command's output) or **not applicable**. Add anything new you find.

Re-run the Linux spike for the provider tools, in a sandbox: install and uninstall one plugin with `claude plugin` and one with `codex plugin`, install and remove one skill with `npx skills add -g` / `npx skills remove -g`, and diff the files and configs before and after each. `PLAN.md` lists what each tool left behind on Linux.

Done when every checklist item has a status and the spike's diffs are recorded in the PR description.

### 3. Implement

Fix what the audit found, one concern per commit, each with a test where the logic is non-trivial. Keep Linux behavior unchanged.

Done when `mise run ci` passes on Windows and on Linux (CI covers Linux).

### 4. CI and release

Add a Windows job to `.github/workflows/ci.yml`, Windows rows to `.github/workflows/release.yml`, and Windows packaging (a `.zip` with the `.exe`, `LICENSE`, `THIRD_PARTY_LICENSES.md`, the font and color-scheme licenses). Add the Windows target triples to `TARGETS` in `scripts/licenses.py` and run `mise run licenses`.

Done when the PR's CI is green on both OSes and `mise run package x86_64-pc-windows-msvc` produces the zip locally.

### 5. Verify with the user

With the user's go-ahead, walk through the app against a sandbox profile: list skills and plugins, toggle a skill per provider, uninstall a skill, install one from Discover → Skills, save and remove a GitHub token in Settings, open the Licenses page.

Done when each flow works and its result on disk matches what the confirm dialog promised. Then update `PLAN.md` (provider facts for Windows, milestone status), update `README.md` (Windows install), delete this file, and open the PR.

## Audit checklist

### Build and packaging

- **Shader compiler.** `fxc.exe` from the Windows SDK (see Toolchain). Check whether GitHub's `windows-latest` and `windows-11-arm` runners have it.
- **Console window.** `crates/skillshelf/src/main.rs` has no `windows_subsystem` attribute, so the app opens a console window next to its own. Use `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` or similar.
- **App icon.** Windows takes the taskbar and Start-menu icon from a resource in the `.exe`. Make `assets/brand/skillshelf.ico` (16 to 256 px) from `assets/brand/skillshelf.svg` and embed it with a build script. `gpui-pre` already embeds its own manifest through `embed-resource` (its default `windows-manifest` feature), so check that the icon resource and gpui's manifest link together.
- **Packaging.** The `package` task in `mise.toml` is a bash script making a `.tar.gz` with `sha256sum`. Windows needs a `.zip` and a SHA-256 file (PowerShell has `Compress-Archive` and `Get-FileHash`).
- **Line endings.** With `core.autocrlf`, files check out with CRLF. Confirm `mise run licenses-check` and `cargo fmt --check` still pass; add a `.gitattributes` (`* text=auto eol=lf`) if they don't.

### Running provider tools (`crates/skillshelf-core/src/env.rs`)

- **`.cmd` shims.** `Command::new("claude")` only finds `claude.exe`. npm installs put `claude.cmd` and `codex.cmd` on `PATH`, and Claude Code's native installer puts `claude.exe` in `%USERPROFILE%\.local\bin`. Resolve the tool through `PATH` and `PATHEXT`. Rust (1.77 and later) escapes arguments for `.cmd`/`.bat` targets and refuses ones it can't escape safely; plugin ids are passed as arguments, so keep that path working.
- **Console flashes.** A GUI-subsystem process that starts console programs (`claude`, `codex`, `gh`) shows a console window for each one. Set `CREATE_NO_WINDOW` (`0x08000000`) with `std::os::windows::process::CommandExt::creation_flags` on every spawn.
- **`PATH` lookup.** `login_shell_path()` reads `$SHELL`, which Windows doesn't set, so it falls back to the process `PATH`. Confirm that a Start-menu launch finds the tools.

### Filesystem (`crates/skillshelf-core/src/ops.rs`, `skills.rs`, `config.rs`, `discover.rs`)

- **Links.** `symlink()` in `ops.rs` returns an error on non-Unix. `npx skills` uses directory junctions on Windows, with absolute targets, because they need neither admin rights nor Developer Mode; it falls back to copying when that fails. Do the same. The standard library has no junction API, so the `junction` crate is the likely choice.
- **Link targets.** `claude_link_target()` in `ops.rs` returns the relative `../../.agents/skills/<name>`. Junctions need an absolute target.
- **Removing links.** `remove()` in `ops.rs` calls `remove_file` on anything `symlink_metadata` doesn't report as a directory. On Windows a directory symlink or junction needs `fs::remove_dir`. It must remove the link and never touch its target: add a test with a junction pointing at a folder outside the sandbox.
- **Comparing paths.** `same_dir()` in `skills.rs` compares `fs::canonicalize` results, which are `\\?\` paths on Windows. Confirm that a junction and its target compare equal.
- **Copies from `npx skills`.** When `npx skills` falls back to copying, `~\.claude\skills\<name>` is a real folder, which skillshelf currently labels "unmanaged". Decide whether a copy that matches a lock entry counts as managed.
- **Replacing config files.** `write_atomic()` in `config.rs` renames a temp file over the target. That works on Windows, but fails with a sharing violation while another process holds the file open. Check what happens while Claude Code or Codex is running, and make the error readable.
- **Codex skill paths.** `[[skills.config]]` entries are written and matched as exact strings (`set_codex_skill`, `codex_disabled_skills` in `config.rs`). Check how Codex itself writes these paths on Windows (separators, drive-letter case) and match the way it does.
- **Extracting downloads.** `extract()` in `discover.rs` checks each path component on the platform's `Path`, so a `..\` inside a tar entry name should be refused on Windows. Add a test for that, and refuse names Windows can't store (`:`, `*`, `?`, `"`, `<`, `>`, `|`, trailing dots or spaces, `CON`, `NUL`, `AUX`, `COM1`…) with a clear error instead of a half-extracted skill.
- **Shown paths.** `Env::tilde()` prints `~/` followed by a path with backslashes. Pick one style for Windows.

### skillshelf's own settings (`crates/skillshelf/src/settings.rs`)

- **Location.** `config_dir()` uses `XDG_CONFIG_HOME` or `~/.config/skillshelf`. On Windows this belongs in `%APPDATA%\skillshelf`.
- **Token file.** The GitHub token is written with mode `0600` on Unix only. On Windows it relies on the profile folder's ACL being private to the user; confirm that, and say so in the Settings page text if it holds.

### Providers (the tables in `PLAN.md` were verified on Linux)

Confirm on Windows, and record the actual paths:

- Claude Code: `%USERPROFILE%\.claude\skills`, `settings.json` (`skillOverrides`, `enabledPlugins`), `plugins\installed_plugins.json`, the plugin cache and its `.in_use` PID files, and `claude plugin list --json` / `uninstall --keep-data`.
- Codex: `%USERPROFILE%\.codex` (or `CODEX_HOME`), whether it reads `%USERPROFILE%\.agents\skills`, `config.toml` keys, `codex plugin list --json` / `remove`, and the `[hooks.state]` leftovers.
- `npx skills`: the canonical folder, the lock file, and junction versus copy.
- `gh auth token`.

### Tests

- **They don't compile on Windows yet.** The test modules in `skills.rs`, `ops.rs` and `discover.rs` call `std::os::unix::fs::symlink` outside any `cfg(unix)`. Add a shared test helper that creates a junction on Windows and a symlink on Unix.
- Run the network tests too: `cargo test -p skillshelf-core -- --ignored` (wrap it in a mise task if you use it more than once).

### UI

- `gpui-component`'s `TitleBar` draws its own minimize, maximize and close buttons on Windows. Check that they and window dragging work.
- Check scaling at 150% and 200%, and that the embedded Geist fonts render.

### CI, release, docs

- `ci.yml`: add a `windows-latest` job running `mise run ci` (it has no `apt-get` step).
- `release.yml`: add `x86_64-pc-windows-msvc` on `windows-latest` and `aarch64-pc-windows-msvc` on `windows-11-arm`, with the zip packaging.
- `scripts/licenses.py`: add both Windows triples to `TARGETS`, then run `mise run licenses`. Windows-only crates (`windows-sys`, `windows-core`…) then get credited.
- `README.md`: Windows install steps. The binary is unsigned, so SmartScreen will warn on first launch; code signing is out of scope here, but mention it in the PR as a follow-up.
