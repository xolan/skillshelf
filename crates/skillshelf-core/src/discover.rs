//! Finding new skills on skills.sh and fetching them from GitHub.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::read_json;
use crate::ops::claude_link_target;
use crate::skills::frontmatter;
use crate::{Action, Env, Provider};

const USER_AGENT: &str = concat!("skillshelf/", env!("CARGO_PKG_VERSION"));
/// Largest repository archive to download.
const MAX_ARCHIVE: u64 = 200 * 1024 * 1024;
/// Largest skill folder to extract.
const MAX_SKILL: u64 = 20 * 1024 * 1024;

/// A skills.sh search result.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillHit {
    /// `owner/repo/skill`.
    pub id: String,
    /// The GitHub repository, `owner/repo`.
    pub source: String,
    pub skill_id: String,
    pub name: String,
    #[serde(default)]
    pub installs: u64,
}

pub fn search_skills(query: &str) -> Result<Vec<SkillHit>> {
    #[derive(Deserialize)]
    struct Response {
        skills: Vec<SkillHit>,
    }
    let request = || {
        ureq::get("https://skills.sh/api/search")
            .header("User-Agent", USER_AGENT)
            .query("q", query)
            .query("limit", "50")
            .call()
    };
    // skills.sh answers 503 now and then; one retry clears it.
    let response = match request() {
        Err(ureq::Error::StatusCode(code)) if code >= 500 => {
            std::thread::sleep(std::time::Duration::from_millis(500));
            request()
        }
        other => other,
    };
    let response: Response = response
        .context("skills.sh search failed")?
        .body_mut()
        .read_json()
        .context("unexpected skills.sh response")?;
    Ok(response.skills)
}

/// A skill downloaded into a staging directory, ready to install.
#[derive(Clone, Debug)]
pub struct FetchedSkill {
    /// Directory name it installs as.
    pub name: String,
    pub source: String,
    /// The staging directory holding the skill's files.
    pub dir: PathBuf,
    /// Path of its SKILL.md in the repository.
    pub skill_path: String,
    /// Git tree hash of the skill folder, as `npx skills` records it.
    pub folder_hash: String,
    pub skill_md: String,
    /// Every file, relative to the skill folder.
    pub files: Vec<String>,
}

impl FetchedSkill {
    /// Removes the staging directory.
    pub fn discard(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Downloads `skill_id` from the GitHub repository `source` into a staging directory. `token`
/// raises GitHub's API rate limit from 60 to 5,000 requests an hour.
pub fn fetch_skill(
    env: &Env,
    source: &str,
    skill_id: &str,
    token: Option<&str>,
) -> Result<FetchedSkill> {
    let commit = head_commit(source, token)?;
    let tree: Tree = serde_json::from_str(
        &github(
            token,
            &format!("https://api.github.com/repos/{source}/git/trees/{commit}?recursive=1"),
        )
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_ARCHIVE)
        .read_to_string()?,
    )
    .context("unexpected GitHub tree response")?;
    let folder = find_skill_folder(&tree, skill_id, |path| {
        let url = format!("https://raw.githubusercontent.com/{source}/{commit}/{path}");
        Ok(github(token, &url).call()?.body_mut().read_to_string()?)
    })?;
    let folder_hash = if folder.is_empty() {
        tree.sha.clone()
    } else {
        tree.tree
            .iter()
            .find(|e| e.kind == "tree" && e.path == folder)
            .map(|e| e.sha.clone())
            .context("skill folder missing from the GitHub tree")?
    };
    download(env, source, &commit, &folder, folder_hash, skill_id)
}

/// Extracts `folder` of `source` at `commit` into a staging directory. The skill is named by its
/// frontmatter, else `fallback_name`.
fn download(
    env: &Env,
    source: &str,
    commit: &str,
    folder: &str,
    folder_hash: String,
    fallback_name: &str,
) -> Result<FetchedSkill> {
    let dir = staging_root(env).join(unique());
    fs::create_dir_all(&dir)?;
    let archive = github(
        None,
        &format!("https://codeload.github.com/{source}/tar.gz/{commit}"),
    )
    .call()
    .context("could not download the repository")?
    .into_body()
    .into_with_config()
    .limit(MAX_ARCHIVE)
    .reader();
    let files = match extract(archive, folder, &dir) {
        Ok(files) => files,
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
    };

    let skill_md = fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
    let name = frontmatter(&skill_md)
        .name
        .filter(|n| valid_name(n))
        .unwrap_or_else(|| fallback_name.to_owned());
    ensure!(valid_name(&name), "unsafe skill name: {name}");
    let skill_path = if folder.is_empty() {
        "SKILL.md".to_owned()
    } else {
        format!("{folder}/SKILL.md")
    };
    Ok(FetchedSkill {
        name,
        source: source.to_owned(),
        dir,
        skill_path,
        folder_hash,
        skill_md,
        files,
    })
}

/// Moves a fetched skill into `~/.agents/skills`, records it in the lock file, and exposes it to
/// the chosen `providers` out of the `offered` ones. Fails if the name is already taken.
pub fn install_skill(
    env: &Env,
    skill: &FetchedSkill,
    providers: &[Provider],
    offered: &[Provider],
) -> Result<Vec<Action>> {
    let target = env.agents_skills().join(&skill.name);
    let link = env.claude_skills().join(&skill.name);
    ensure!(
        fs::symlink_metadata(&target).is_err(),
        "{} already exists",
        env.tilde(&target)
    );
    let link_claude = providers.contains(&Provider::Claude);
    ensure!(
        !link_claude || fs::symlink_metadata(&link).is_err(),
        "{} already exists",
        env.tilde(&link)
    );
    let now = humantime::format_rfc3339_millis(SystemTime::now()).to_string();
    let mut actions = vec![
        Action::Rename {
            from: skill.dir.clone(),
            to: target.clone(),
        },
        Action::AddLock {
            name: skill.name.clone(),
            entry: json!({
                "source": skill.source,
                "sourceType": "github",
                "sourceUrl": format!("https://github.com/{}.git", skill.source),
                "skillPath": skill.skill_path,
                "skillFolderHash": skill.folder_hash,
                "installedAt": now,
                "updatedAt": now,
            }),
        },
    ];
    if link_claude {
        actions.push(Action::Link {
            link,
            target: claude_link_target(env, &target, &skill.name),
        });
    }
    // Codex reads ~/.agents/skills on its own, so leaving it out means turning it off there. A
    // provider that wasn't offered is not managed here, and its config is left alone.
    if offered.contains(&Provider::Codex) && !providers.contains(&Provider::Codex) {
        actions.push(Action::SetCodexSkill {
            skill_md: target.join("SKILL.md"),
            enabled: false,
        });
    }
    Ok(actions)
}

/// A newer version of an installed skill on GitHub.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillUpdate {
    /// Directory name of the installed skill.
    pub name: String,
    pub source: String,
    /// The skill's folder in the repository, `""` for the root.
    pub folder: String,
    /// Git tree hash of the folder on GitHub now.
    pub folder_hash: String,
}

/// Compares the folder hash the lock file recorded at install with the folder on GitHub now.
/// `None` when they match, or the skill was not installed from GitHub.
pub fn check_skill_update(
    env: &Env,
    name: &str,
    token: Option<&str>,
) -> Result<Option<SkillUpdate>> {
    let Some((source, folder, installed)) = locked_skill(&read_json(&env.skill_lock())?, name)
    else {
        return Ok(None);
    };
    let folder_hash = tree_hash(&source, "HEAD", &folder, token)?;
    Ok((folder_hash != installed).then(|| SkillUpdate {
        name: name.to_owned(),
        source,
        folder,
        folder_hash,
    }))
}

/// `(source, folder, skillFolderHash)` of a skill the lock file says came from GitHub.
fn locked_skill(lock: &Value, name: &str) -> Option<(String, String, String)> {
    let entry = &lock["skills"][name];
    if entry["sourceType"] != "github" {
        return None;
    }
    let source = entry["source"].as_str()?;
    let path = entry["skillPath"].as_str()?;
    let folder = path.strip_suffix("SKILL.md")?.trim_end_matches('/');
    let hash = entry["skillFolderHash"]
        .as_str()
        .filter(|h| !h.is_empty())?;
    Some((source.to_owned(), folder.to_owned(), hash.to_owned()))
}

/// Downloads the newer version of an installed skill into a staging directory.
pub fn fetch_update(env: &Env, update: &SkillUpdate, token: Option<&str>) -> Result<FetchedSkill> {
    let commit = head_commit(&update.source, token)?;
    let folder_hash = tree_hash(&update.source, &commit, &update.folder, token)?;
    download(
        env,
        &update.source,
        &commit,
        &update.folder,
        folder_hash,
        &update.name,
    )
}

/// Swaps the shared copy of skill `name` for a downloaded newer version and records the new
/// folder hash in the lock file. The old copy is moved aside first and deleted last, so a failed
/// step never leaves a half-deleted skill.
pub fn update_skill(env: &Env, name: &str, skill: &FetchedSkill) -> Result<Vec<Action>> {
    let target = env.agents_skills().join(name);
    ensure!(
        fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()),
        "{} is not a folder skillshelf can replace",
        env.tilde(&target)
    );
    let mut entry = read_json(&env.skill_lock())?["skills"][name].clone();
    ensure!(entry.is_object(), "{name} is not in the skill lock");
    entry["skillPath"] = skill.skill_path.clone().into();
    entry["skillFolderHash"] = skill.folder_hash.clone().into();
    entry["updatedAt"] = humantime::format_rfc3339_millis(SystemTime::now())
        .to_string()
        .into();
    let old = staging_root(env).join(format!("{}-old", unique()));
    let mut actions = vec![
        Action::Rename {
            from: target.clone(),
            to: old.clone(),
        },
        Action::Rename {
            from: skill.dir.clone(),
            to: target.clone(),
        },
        Action::AddLock {
            name: name.to_owned(),
            entry,
        },
    ];
    // A copy in ~/.claude/skills, made where `npx skills` could not link, would keep the old
    // version.
    let claude_copy = env.claude_skills().join(name);
    if fs::symlink_metadata(&claude_copy).is_ok_and(|m| m.is_dir()) {
        actions.push(Action::Remove(claude_copy.clone()));
        actions.push(Action::Copy {
            from: target,
            to: claude_copy,
        });
    }
    actions.push(Action::Remove(old));
    Ok(actions)
}

/// Downloads wait here: next to `~/.agents/skills`, so installing is a rename, but outside it,
/// so Codex never loads a skill that is only being previewed.
pub(crate) fn staging_root(env: &Env) -> PathBuf {
    env.home.join(".agents").join(".skillshelf-staging")
}

/// Removes downloads left by an earlier run.
pub fn clean_staging(env: &Env) {
    let _ = fs::remove_dir_all(staging_root(env));
}

#[derive(Deserialize)]
struct Tree {
    sha: String,
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

/// The folder (`""` for the repository root) holding the skill: a SKILL.md folder named
/// `skill_id`, else the only one there is, else the one whose frontmatter names it.
fn find_skill_folder(
    tree: &Tree,
    skill_id: &str,
    read: impl Fn(&str) -> Result<String>,
) -> Result<String> {
    let folders: Vec<&str> = tree
        .tree
        .iter()
        .filter(|e| e.kind == "blob")
        .filter_map(|e| {
            if e.path == "SKILL.md" {
                Some("")
            } else {
                e.path.strip_suffix("/SKILL.md")
            }
        })
        .collect();
    let by_dir: Vec<&str> = folders
        .iter()
        .copied()
        .filter(|f| f.rsplit('/').next() == Some(skill_id))
        .collect();
    if let [folder] = by_dir[..] {
        return Ok(folder.to_owned());
    }
    if let [folder] = folders[..] {
        return Ok(folder.to_owned());
    }
    let candidates = if by_dir.is_empty() { &folders } else { &by_dir };
    for folder in candidates.iter().take(50) {
        let path = if folder.is_empty() {
            "SKILL.md".to_owned()
        } else {
            format!("{folder}/SKILL.md")
        };
        if frontmatter(&read(&path)?).name.as_deref() == Some(skill_id) {
            return Ok((*folder).to_owned());
        }
    }
    if tree.truncated {
        bail!("the repository is too large to search for {skill_id}");
    }
    bail!("no skill named {skill_id} in the repository")
}

/// Extracts the files under `folder` from a GitHub tarball into `dest`. Refuses links and any
/// path that would leave `dest`.
fn extract(archive: impl Read, folder: &str, dest: &Path) -> Result<Vec<String>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let folder: Vec<String> = folder
        .split('/')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    let mut files = Vec::new();
    let mut total = 0;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        // GitHub wraps everything in one `<repo>-<commit>/` directory.
        let parts: Vec<String> = path
            .components()
            .skip(1)
            .map(|c| match c {
                Component::Normal(part) => Ok(part.to_string_lossy().into_owned()),
                _ => bail!("unsafe path in archive: {}", path.display()),
            })
            .collect::<Result<_>>()?;
        let Some(rel) = parts.strip_prefix(&folder[..]) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        ensure!(
            rel.iter().all(|part| storable(part)),
            "the skill has a file name Windows can't store: {}",
            rel.join("/")
        );
        let out = rel.iter().fold(dest.to_path_buf(), |p, part| p.join(part));
        match entry.header().entry_type() {
            tar::EntryType::Directory => fs::create_dir_all(&out)?,
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                total += entry.size();
                ensure!(
                    total <= MAX_SKILL,
                    "skill is larger than {} MB",
                    MAX_SKILL >> 20
                );
                fs::create_dir_all(out.parent().context("no parent")?)?;
                entry.unpack(&out)?;
                files.push(rel.join("/"));
            }
            tar::EntryType::Symlink | tar::EntryType::Link => {
                bail!(
                    "skill contains a link ({}); refusing to install it",
                    rel.join("/")
                )
            }
            _ => {}
        }
    }
    ensure!(
        files.iter().any(|f| f == "SKILL.md"),
        "the download has no SKILL.md"
    );
    files.sort();
    Ok(files)
}

/// Whether Windows can store a file named `name` as it is; elsewhere every name is fine. Windows
/// would fail on the others halfway through, or worse: `C:` or `name:stream` lead out of the
/// folder or into a hidden stream, and a trailing dot or space is dropped, so `SKILL.md.` would
/// overwrite `SKILL.md`.
fn storable(name: &str) -> bool {
    const DEVICES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
        "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2",
        "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
    ];
    if !cfg!(windows) {
        return true;
    }
    // Device names are taken with any extension too (`nul.txt`) on Windows 10.
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c))
        && !DEVICES
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// Refuses anything but `owner/repo`, since it goes into GitHub URLs.
fn check_source(source: &str) -> Result<()> {
    ensure!(
        source.split('/').count() == 2 && source.split('/').all(valid_name),
        "not a GitHub repository: {source}"
    );
    Ok(())
}

/// The commit `HEAD` of a GitHub repository points to.
fn head_commit(source: &str, token: Option<&str>) -> Result<String> {
    check_source(source)?;
    let commit = github(
        token,
        &format!("https://api.github.com/repos/{source}/commits/HEAD"),
    )
    .header("Accept", "application/vnd.github.sha")
    .call()
    .with_context(|| format!("could not reach github.com/{source}"))?
    .body_mut()
    .read_to_string()?
    .trim()
    .to_owned();
    ensure!(
        commit.chars().all(|c| c.is_ascii_hexdigit()),
        "unexpected commit id from GitHub"
    );
    Ok(commit)
}

/// Git tree hash of `folder` (`""` for the root) at `rev`, one small API call.
fn tree_hash(source: &str, rev: &str, folder: &str, token: Option<&str>) -> Result<String> {
    #[derive(Deserialize)]
    struct Tree {
        sha: String,
    }
    check_source(source)?;
    ensure!(
        folder.is_empty()
            || folder
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".." && !p.contains(['?', '#', '%'])),
        "unsafe skill folder: {folder}"
    );
    let rev = if folder.is_empty() {
        rev.to_owned()
    } else {
        format!("{rev}:{folder}")
    };
    let tree: Tree = github(
        token,
        &format!("https://api.github.com/repos/{source}/git/trees/{rev}"),
    )
    .call()
    .with_context(|| format!("could not read {folder} in github.com/{source}"))?
    .body_mut()
    .read_json()
    .context("unexpected GitHub tree response")?;
    Ok(tree.sha)
}

fn github(token: Option<&str>, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    let request = ureq::get(url).header("User-Agent", USER_AGENT);
    // Only the API needs the token; never send it anywhere else.
    match token {
        Some(token) if url.starts_with("https://api.github.com/") => {
            request.header("Authorization", format!("Bearer {token}"))
        }
        _ => request,
    }
}

/// The token the GitHub CLI is signed in with (`gh auth token`).
pub fn gh_token(env: &Env) -> Result<String> {
    let token = env.run("gh", &["auth".into(), "token".into()])?;
    let token = token.trim();
    ensure!(!token.is_empty(), "gh is not signed in");
    Ok(token.to_owned())
}

/// GitHub's core API rate limit for `token` (or for anonymous use): `(limit, remaining)` per hour.
/// Checking it does not count against the limit.
pub fn github_rate_limit(token: Option<&str>) -> Result<(u64, u64)> {
    #[derive(Deserialize)]
    struct Core {
        limit: u64,
        remaining: u64,
    }
    #[derive(Deserialize)]
    struct Resources {
        core: Core,
    }
    #[derive(Deserialize)]
    struct Response {
        resources: Resources,
    }
    let response: Response = match github(token, "https://api.github.com/rate_limit").call() {
        Err(ureq::Error::StatusCode(401)) => {
            bail!("GitHub rejected the token: it is invalid or has expired")
        }
        result => result.context("could not reach GitHub")?,
    }
    .body_mut()
    .read_json()
    .context("unexpected GitHub response")?;
    Ok((
        response.resources.core.limit,
        response.resources.core.remaining,
    ))
}

fn unique() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

/// Adds or replaces a skill's entry in the lock file, creating the file if needed.
pub(crate) fn add_lock_entry(lock: &mut Value, name: &str, entry: &Value) -> Result<()> {
    let root = lock
        .as_object_mut()
        .context("the skill lock is not a JSON object")?;
    root.entry("version").or_insert(json!(3));
    let skills = root
        .entry("skills")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`skills` in the skill lock is not an object")?;
    skills.insert(name.to_owned(), entry.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> Tree {
        Tree {
            sha: "root".into(),
            tree: paths
                .iter()
                .map(|p| TreeEntry {
                    path: (*p).into(),
                    kind: "blob".into(),
                    sha: "x".into(),
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn finds_skill_folder() {
        let no_read = |_: &str| -> Result<String> { panic!("should not read") };
        let t = tree(&["skills/a/SKILL.md", "skills/b/SKILL.md", "README.md"]);
        assert_eq!(find_skill_folder(&t, "b", no_read).unwrap(), "skills/b");
        assert_eq!(
            find_skill_folder(&tree(&["SKILL.md"]), "x", no_read).unwrap(),
            ""
        );
        let by_name = |path: &str| -> Result<String> {
            Ok(if path == "skills/b/SKILL.md" {
                "---\nname: renamed\n---\n"
            } else {
                ""
            }
            .into())
        };
        assert_eq!(
            find_skill_folder(&t, "renamed", by_name).unwrap(),
            "skills/b"
        );
        assert!(find_skill_folder(&t, "missing", |_| Ok(String::new())).is_err());
    }

    fn tarball(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("../../etc/passwd").unwrap();
            }
            // Bypass the builder's path checks so unsafe paths reach the extractor.
            let name = &mut header.as_old_mut().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extracts_only_the_skill_folder() {
        let dest = tempfile::tempdir().unwrap();
        let archive = tarball(&[
            ("repo-abc/README.md", tar::EntryType::Regular, b"x"),
            (
                "repo-abc/skills/a/SKILL.md",
                tar::EntryType::Regular,
                b"---\nname: a\n---\n",
            ),
            (
                "repo-abc/skills/a/scripts/run.sh",
                tar::EntryType::Regular,
                b"echo",
            ),
            (
                "repo-abc/skills/ab/SKILL.md",
                tar::EntryType::Regular,
                b"other",
            ),
        ]);
        let files = extract(&archive[..], "skills/a", dest.path()).unwrap();
        assert_eq!(files, ["SKILL.md", "scripts/run.sh"]);
        assert!(dest.path().join("scripts/run.sh").is_file());
        assert!(!dest.path().join("README.md").exists());
    }

    #[test]
    fn refuses_links_and_traversal() {
        let dest = tempfile::tempdir().unwrap();
        let link = tarball(&[
            ("repo-abc/s/SKILL.md", tar::EntryType::Regular, b"x"),
            ("repo-abc/s/evil", tar::EntryType::Symlink, b""),
        ]);
        assert!(extract(&link[..], "s", dest.path()).is_err());
        let traversal = tarball(&[("repo-abc/s/../../evil", tar::EntryType::Regular, b"x")]);
        assert!(extract(&traversal[..], "s", dest.path()).is_err());
        assert!(!dest.path().parent().unwrap().join("evil").exists());
    }

    #[test]
    fn refuses_names_windows_cannot_store_on_windows() {
        for name in [
            "C:/evil",
            "SKILL.md:hidden",
            "CON",
            "nul.txt",
            "a?b",
            "trail.",
            "trail ",
        ] {
            let dest = tempfile::tempdir().unwrap();
            let archive = tarball(&[
                ("repo-abc/s/SKILL.md", tar::EntryType::Regular, b"x"),
                (&format!("repo-abc/s/{name}"), tar::EntryType::Regular, b"x"),
            ]);
            let result = extract(&archive[..], "s", dest.path());
            assert_eq!(result.is_err(), cfg!(windows), "{name}");
        }
        // `C:evil` would be relative to the current directory on drive C.
        assert!(!Path::new("C:evil").exists());
        // Names elsewhere in the repository don't matter.
        let dest = tempfile::tempdir().unwrap();
        let archive = tarball(&[
            ("repo-abc/s/SKILL.md", tar::EntryType::Regular, b"x"),
            ("repo-abc/other/CON", tar::EntryType::Regular, b"x"),
        ]);
        assert!(extract(&archive[..], "s", dest.path()).is_ok());
    }

    #[test]
    fn install_plans_link_and_codex_opt_out() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let skill = FetchedSkill {
            name: "s".into(),
            source: "o/r".into(),
            dir: staging_root(&env).join("1"),
            skill_path: "skills/s/SKILL.md".into(),
            folder_hash: "abc".into(),
            skill_md: String::new(),
            files: vec!["SKILL.md".into()],
        };
        let actions = install_skill(&env, &skill, &[Provider::Claude], &Provider::ALL).unwrap();
        assert_eq!(actions.len(), 4);
        let target = if cfg!(windows) {
            env.agents_skills().join("s")
        } else {
            "../../.agents/skills/s".into()
        };
        assert_eq!(
            actions[2],
            Action::Link {
                link: env.claude_skills().join("s"),
                target
            }
        );
        assert_eq!(
            actions[3],
            Action::SetCodexSkill {
                skill_md: env.agents_skills().join("s/SKILL.md"),
                enabled: false
            }
        );
        // With Codex not managed, its config is not touched.
        let claude_only = [Provider::Claude];
        let actions = install_skill(&env, &skill, &claude_only, &claude_only).unwrap();
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::SetCodexSkill { .. }))
        );
        fs::create_dir_all(env.agents_skills().join("s")).unwrap();
        assert!(install_skill(&env, &skill, &Provider::ALL, &Provider::ALL).is_err());
    }

    #[test]
    fn lock_entry_keeps_other_fields() {
        let mut lock = json!({"version": 3, "skills": {"a": {}}, "dismissed": {}});
        add_lock_entry(&mut lock, "b", &json!({"source": "o/r"})).unwrap();
        assert_eq!(
            lock,
            json!({"version": 3, "skills": {"a": {}, "b": {"source": "o/r"}}, "dismissed": {}})
        );
        let mut empty = json!({});
        add_lock_entry(&mut empty, "b", &json!({})).unwrap();
        assert_eq!(empty, json!({"version": 3, "skills": {"b": {}}}));
    }

    #[test]
    fn reads_github_skills_from_the_lock() {
        let entry = |path: &str| json!({"source": "o/r", "sourceType": "github", "skillPath": path, "skillFolderHash": "h"});
        let lock = json!({"skills": {
            "nested": entry("skills/a/SKILL.md"),
            "root": entry("SKILL.md"),
            "local": {"source": "/x", "sourceType": "local", "skillPath": "SKILL.md", "skillFolderHash": "h"},
            "no-hash": {"source": "o/r", "sourceType": "github", "skillPath": "SKILL.md", "skillFolderHash": ""},
        }});
        let got = |name| locked_skill(&lock, name);
        let expect = |folder: &str| Some(("o/r".into(), folder.into(), "h".into()));
        assert_eq!(got("nested"), expect("skills/a"));
        assert_eq!(got("root"), expect(""));
        assert_eq!(got("local"), None);
        assert_eq!(got("no-hash"), None);
        assert_eq!(got("missing"), None);
    }

    #[test]
    fn update_swaps_the_shared_copy() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let target = env.agents_skills().join("s");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "old").unwrap();
        fs::write(target.join("gone.md"), "x").unwrap();
        fs::write(
            env.skill_lock(),
            r#"{"version":3,"skills":{"s":{"source":"o/r","sourceType":"github","skillPath":"s/SKILL.md","skillFolderHash":"old","installedAt":"then"}}}"#,
        )
        .unwrap();
        fs::create_dir_all(env.claude_skills()).unwrap();
        crate::testing::link_dir("../../.agents/skills/s", env.claude_skills().join("s"));
        let dir = staging_root(&env).join("new");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "new").unwrap();
        let fetched = FetchedSkill {
            name: "renamed".into(),
            source: "o/r".into(),
            dir,
            skill_path: "s/SKILL.md".into(),
            folder_hash: "new".into(),
            skill_md: "new".into(),
            files: vec!["SKILL.md".into()],
        };

        let actions = update_skill(&env, "s", &fetched).unwrap();
        crate::execute(&env, &actions).unwrap();
        assert_eq!(fs::read_to_string(target.join("SKILL.md")).unwrap(), "new");
        assert!(!target.join("gone.md").exists());
        assert_eq!(
            fs::read_to_string(env.claude_skills().join("s/SKILL.md")).unwrap(),
            "new"
        );
        assert_eq!(fs::read_dir(staging_root(&env)).unwrap().count(), 0);
        let entry = &read_json(&env.skill_lock()).unwrap()["skills"]["s"];
        assert_eq!(entry["skillFolderHash"], "new");
        assert_eq!(entry["installedAt"], "then");
        assert_ne!(entry["updatedAt"], Value::Null);
    }

    #[test]
    fn update_refreshes_a_copy_in_claude() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        for dir in [env.agents_skills().join("s"), env.claude_skills().join("s")] {
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("SKILL.md"), "old").unwrap();
        }
        fs::write(
            env.skill_lock(),
            r#"{"skills":{"s":{"skillPath":"SKILL.md"}}}"#,
        )
        .unwrap();
        let dir = staging_root(&env).join("new");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "new").unwrap();
        let fetched = FetchedSkill {
            name: "s".into(),
            source: "o/r".into(),
            dir,
            skill_path: "SKILL.md".into(),
            folder_hash: "new".into(),
            skill_md: "new".into(),
            files: vec!["SKILL.md".into()],
        };
        crate::execute(&env, &update_skill(&env, "s", &fetched).unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(env.claude_skills().join("s/SKILL.md")).unwrap(),
            "new"
        );
        assert_eq!(fs::read_dir(staging_root(&env)).unwrap().count(), 0);
    }

    #[test]
    fn update_never_replaces_a_link() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let dotfiles = home.path().join("dotfiles/s");
        fs::create_dir_all(&dotfiles).unwrap();
        fs::create_dir_all(env.agents_skills()).unwrap();
        crate::testing::link_dir(&dotfiles, env.agents_skills().join("s"));
        fs::write(env.skill_lock(), r#"{"skills":{"s":{}}}"#).unwrap();
        let fetched = FetchedSkill {
            name: "s".into(),
            source: "o/r".into(),
            dir: staging_root(&env).join("new"),
            skill_path: "SKILL.md".into(),
            folder_hash: "new".into(),
            skill_md: String::new(),
            files: Vec::new(),
        };
        assert!(update_skill(&env, "s", &fetched).is_err());
    }

    /// Finds an outdated skill on GitHub and downloads its update into a throwaway home.
    #[test]
    #[ignore = "uses the network"]
    fn check_and_fetch_update() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        fs::create_dir_all(home.path().join(".agents")).unwrap();
        fs::write(
            env.skill_lock(),
            r#"{"skills":{"find-skills":{"source":"vercel-labs/skills","sourceType":"github","skillPath":"skills/find-skills/SKILL.md","skillFolderHash":"0000"}}}"#,
        )
        .unwrap();
        let update = check_skill_update(&env, "find-skills", None)
            .unwrap()
            .unwrap();
        assert_eq!(update.folder, "skills/find-skills");
        assert_eq!(update.folder_hash.len(), 40);
        let fetched = fetch_update(&env, &update, None).unwrap();
        assert_eq!(fetched.folder_hash, update.folder_hash);
        assert!(fetched.files.contains(&"SKILL.md".to_owned()));
    }

    /// Searches skills.sh and installs a real skill into a throwaway home.
    #[test]
    #[ignore = "uses the network"]
    fn search_fetch_and_install() {
        let hit = search_skills("find-skills")
            .unwrap()
            .into_iter()
            .find(|h| h.source == "vercel-labs/skills" && h.skill_id == "find-skills")
            .expect("skills.sh lists vercel-labs/skills find-skills");
        let home = tempfile::tempdir().unwrap();
        let env = Env::at(home.path());
        let fetched = fetch_skill(&env, &hit.source, &hit.skill_id, None).unwrap();
        assert_eq!(fetched.name, "find-skills");
        assert_eq!(fetched.folder_hash.len(), 40);
        assert!(fetched.files.contains(&"SKILL.md".to_owned()));
        let actions = install_skill(&env, &fetched, &Provider::ALL, &Provider::ALL).unwrap();
        crate::execute(&env, &actions).unwrap();
        let skills = crate::scan_skills(&env).unwrap();
        assert_eq!(skills[0].source.as_deref(), Some("vercel-labs/skills"));
        assert_eq!(skills[0].enabled(Provider::Claude), Some(true));
        assert_eq!(skills[0].enabled(Provider::Codex), Some(true));
    }

    #[test]
    #[ignore = "uses the network"]
    fn anonymous_rate_limit() {
        let (limit, remaining) = github_rate_limit(None).unwrap();
        assert_eq!(limit, 60);
        assert!(remaining <= limit);
    }
}
