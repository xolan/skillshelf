# skillshelf — plan

A native Rust desktop app (GPUI) for managing agent skills and plugins for Claude Code and Codex CLI.
Linux first; Windows and macOS later. Global (user-level) scope only.

## Decisions

- **Name:** `skillshelf` (binary and crate).
- **UI:** GPUI via `gpui-component` (pinned exact version). Own visual identity, Zed-inspired. Not native-looking.
- **Skill discovery:** skills.sh search API.
- **Plugin discovery:** provider marketplaces (`claude plugin list --available --json`, `codex plugin list --available --json`). Separate "Discover → Plugins" section, not mixed with skills.
- **Canonical skill location:** `~/.agents/skills/<name>/`. Share `~/.agents/.skill-lock.json` (v3) with `npx skills`, so both tools see the same state.
- **Toggles:** every skill and plugin can be enabled/disabled per provider, or for all providers at once ("global").
- **Codex's own directory:** skills in `~/.codex/skills/` are listed and can be toggled and uninstalled too.
- **Themes:** own light and dark themes, bundled Base16 schemes, plus user Base16 files in
  `~/.config/skillshelf/themes/`. The choice is saved in `~/.config/skillshelf/settings.json`.
- **Tooling:** mise pins the toolchain; every developer command is a `mise run` task, CI included.

## Provider facts (verified on a real machine, 2026-10-26)

### Skills

| | Claude Code | Codex CLI |
|---|---|---|
| Where it reads user skills | `~/.claude/skills/<name>/` | `~/.agents/skills/<name>/` (native) |
| How skillshelf exposes a skill | relative symlink `../../.agents/skills/<name>` | nothing needed |
| Disable | `~/.claude/settings.json` → `skillOverrides.<name> = "off"` | `~/.codex/config.toml` → `[[skills.config]] path = ".../SKILL.md"`, `enabled = false` |
| Never touch | — | `~/.codex/skills/.system/` (bundled) |

A skill installed for Claude only is hidden from Codex with a `[[skills.config]] enabled = false` entry,
because Codex sees every skill in `~/.agents/skills`.

### What the provider tools leave behind (spike, sandbox HOME)

- `claude plugin uninstall` removes the registry entry and `enabledPlugins` key, deletes plugin data
  unless `--keep-data`, but keeps the cache dir and marks it `.orphaned_at`. Running sessions list
  their PIDs in the version dir's `.in_use/`; skillshelf deletes the cache only when that is empty.
- `codex plugin remove` deletes the cache and `[plugins."id"]`, but keeps `[hooks.state."id:*"]`.
- `codex plugin list --json` includes remote (connector) plugins that are not in `config.toml`, so
  installed plugins are read from each tool's `list --json`, not from files.
- Provider tools run from the home directory: a project `mise.toml` must not pick the tool version.

### Plugins

| | Claude Code | Codex CLI |
|---|---|---|
| Registry | `~/.claude/plugins/installed_plugins.json` (v2) | `~/.codex/config.toml` → `[plugins."name@mkt"]` |
| Enabled flag | `~/.claude/settings.json` → `enabledPlugins."name@mkt"` | `[plugins."name@mkt"] enabled` |
| Files | `~/.claude/plugins/cache/<mkt>/<plugin>/<ver>/` | `~/.codex/plugins/cache/<mkt>/<plugin>/<ver>/` |
| Marketplaces | `~/.claude/plugins/known_marketplaces.json` | `[marketplaces.<name>]` |
| Other state | `~/.claude/plugins/data/` | `[hooks.state."name@mkt:..."]` |
| CLI | `claude plugin list --json`, `install`, `uninstall`, `enable`, `disable` | `codex plugin list --json`, `add`, `remove` |

## Architecture

Cargo workspace:

- `crates/skillshelf-core` — library, no UI. Inventory scan, provider adapters, skills.sh client,
  GitHub fetch, lock file, operation planner and executor. Every path derives from one `Env { home }`,
  so tests run against a temporary home directory.
- `crates/skillshelf` — GPUI binary. Calls core on the background executor.

### Operations

Every mutation is a `Plan` (a list of `Action`s: symlink, remove link, remove dir, edit TOML,
edit JSON, run CLI). The UI shows the plan in a confirm dialog, then executes it. Config files are
edited with `toml_edit` / `serde_json` (`preserve_order`) and written via temp file + rename.
The planner is pure, so it is unit-tested against fixture homes.

### Safety rules

- Only delete what skillshelf manages: dirs under `~/.agents/skills/`, and symlinks that resolve into it.
- Never follow a symlink to delete its target (e.g. dotfile-managed skills).
- Name clash on install → stop and show it; never overwrite.
- Skill archives are untrusted: reject `..`, absolute paths, and symlinks leaving the skill folder.
  Show SKILL.md and the file list (scripts flagged) before installing.

## Flows

### Install skill
1. Search skills.sh (`GET https://skills.sh/api/search?q=<q>&limit=<n>`, debounced).
2. Resolve via GitHub trees API on `source`: find the `SKILL.md` whose frontmatter `name` matches
   `skillId`; record its path and tree SHA (`skillFolderHash`).
3. Download `https://codeload.github.com/<owner>/<repo>/tar.gz/<commit>`, extract only that folder into
   `~/.agents/.skillshelf-staging/` (same filesystem for the final rename, but outside the folder
   Codex scans). Links, `..` paths and folders over 20 MB are refused; script files are flagged.
4. Preview, confirm, rename into `~/.agents/skills/<name>`, add lock entry.
5. Per selected provider: Claude → symlink. Codex → nothing; if Codex not selected, add a disable entry.

### Update skill
1. When a skill with a GitHub lock entry is selected, read its folder's tree hash with one call
   (`GET /repos/<source>/git/trees/HEAD:<folder>`) and compare it with `skillFolderHash`.
   Checked once per skill per session (Reload forgets the results), so the 60 requests/hour
   GitHub allows without a token go far.
2. "Update…" downloads the folder at the current commit into staging like an install, and lists
   its files (scripts flagged) in the confirm dialog.
3. Move the installed folder aside into staging, rename the download into place, update the lock
   entry (hash, `updatedAt`), delete the old copy. Links and Codex/Claude settings keep pointing
   at the same path.

### Uninstall skill
Remove the Claude symlink (only if it points into the canonical dir), `skillOverrides` entry,
`[[skills.config]]` entries, the canonical dir, and the lock entry. Unmanaged skills (real dirs or
foreign symlinks) are removed only as the dir/link itself, after explicit confirmation.

### Uninstall plugin
Per selected provider, run the provider CLI, then sweep leftovers it missed (cache dir, enabled flag,
`[hooks.state."id:*"]`). Optional: delete plugin data. CLIs are looked up beyond `PATH`
(`~/.local/bin`, npm global, mise shims) because GUI launches get a thin `PATH`.

### Toggle skill / plugin
Per provider, or all providers at once. Skills: `skillOverrides` (Claude), `[[skills.config]]` (Codex).
Plugins: `enabledPlugins` (Claude), `[plugins."id"] enabled` (Codex). No files move.

## UI

- Custom title bar, compact rows, keyboard-first, command palette (`ctrl-k`), `/` focuses search.
- Sidebar: **Installed** (Skills, Plugins), **Discover** (Skills via skills.sh, Plugins via marketplaces).
- Main list, right-hand detail pane (SKILL.md / plugin manifest, file tree).
- Each row shows a chip per provider with its state (enabled, disabled, not installed, unmanaged),
  clickable to toggle, plus a global toggle.
- Own theme tokens (light/dark), embedded OFL font, own accent and icon.

## Release

- `ci.yml` on PRs and pushes: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
- `release.yml` on push to `main`: if tag `v<Cargo.toml version>` does not exist, build the matrix,
  package `skillshelf-<ver>-<target>.tar.gz` + `.sha256`, and `gh release create`.
- Targets now: `x86_64-unknown-linux-gnu` (ubuntu-22.04), `aarch64-unknown-linux-gnu` (ubuntu-22.04-arm).
  Later: Windows and macOS rows.

## Milestones

Status (2026-10-26): 0–4 done; 5 done except the app icon and releasing 0.1.0 (the version bump).

0. Spike: sandbox-HOME diffs of `npx skills add/remove -g`, `claude plugin install/uninstall`,
   `codex plugin add/remove` → fixtures and leftover lists. GPUI hello window builds. Release workflow.
1. Core read-only inventory + tests.
2. UI shell: sidebar, lists, detail pane, theme, command palette.
3. Toggles and uninstall (skills, plugins).
4. Discover: skills.sh search + install; marketplace plugin search + install.
5. Polish: app icon, embedded font, command palette (`ctrl-k`), skill updates, keyboard
   navigation in lists. Release 0.1.0.

Later: macOS/Windows (Windows: directory junctions instead of symlinks), project scope, plugin
updates, more providers.
