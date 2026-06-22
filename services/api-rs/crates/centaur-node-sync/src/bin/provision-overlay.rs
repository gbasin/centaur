//! provision-overlay — per-session overlay setup for C4 e2e pods and production.
//!
//! Contract:
//! `provision-overlay --session <id> [--overlays-root /var/lib/centaur/overlays]
//!   [--merged-root /run/centaur/merged] [--lower <dir>] [--harness <kind>]
//!   [--harness-thread-id <id>] [--harness-home <path>] [--repo <path>]
//!   [--agent-uid <uid>]`
//!
//! It prepares the host-backed upper the node-sync daemon scans, creates a seed
//! lower when no repo is provided, mounts the merged workspace using the same
//! overlay options as ci/overlay-validation.sh, and always writes the node-sync
//! sidecar manifest at `<overlays-root>/.sessions/<session>.json`.
//!
//! Lower precedence:
//! - Non-empty `--repo <path>` wins and is mounted as the read-only lowerdir.
//! - Without `--repo`, `--lower <dir>` is the fixture lower override.
//! - Without either, the fixture lower is `<host-root>/overlay-lower/<session>`.

use centaur_node_sync::session_manifest::{SessionManifest, normalize_harness, write_manifest};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
struct Config {
    session: String,
    overlays_root: PathBuf,
    merged_root: PathBuf,
    lower: Option<PathBuf>,
    harness: Option<String>,
    harness_thread_id: String,
    harness_home: String,
    repo: String,
    agent_uid: Option<u32>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LowerKind {
    Fixture,
    Repo,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LowerSource {
    path: PathBuf,
    kind: LowerKind,
}

impl LowerSource {
    fn uses_fixture_seed(&self) -> bool {
        self.kind == LowerKind::Fixture
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("provision-overlay: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cfg = parse_args(std::env::args_os().skip(1))?;
    validate_session(&cfg.session)?;

    let upper = cfg.overlays_root.join(&cfg.session);
    let host_root = cfg
        .overlays_root
        .parent()
        .ok_or_else(|| format!("{} has no parent", cfg.overlays_root.display()))?;
    let work = host_root.join("overlay-work").join(&cfg.session);
    let lower_source =
        select_lower_source(&cfg.repo, cfg.lower.as_deref(), host_root, &cfg.session)?;
    let lower_source = resolve_lower_source(lower_source)?;
    let lower = lower_source.path.clone();
    let merged = cfg.merged_root.join(&cfg.session);

    std::fs::create_dir_all(&upper)
        .map_err(|e| format!("create upper {}: {e}", upper.display()))?;
    if lower_source.uses_fixture_seed() {
        std::fs::create_dir_all(&lower)
            .map_err(|e| format!("create lower {}: {e}", lower.display()))?;
    }
    std::fs::create_dir_all(&merged)
        .map_err(|e| format!("create merged {}: {e}", merged.display()))?;

    match cfg.agent_uid {
        Some(uid) => chown_dir_owner(&upper, uid)?,
        None => {
            // Fixture-grade: the overlay's merged root inherits the UPPER dir's permissions,
            // so make the upper world-writable. Without this the hardened agent (runAsUser
            // 1001) gets "Permission denied" creating files in /workspace.
            set_dir_mode(&upper, 0o777)?;
        }
    }

    let manifest_repo = match lower_source.kind {
        LowerKind::Repo => lower.to_string_lossy().into_owned(),
        LowerKind::Fixture => cfg.repo.clone(),
    };

    write_manifest(
        &cfg.overlays_root,
        &SessionManifest {
            session: cfg.session.clone(),
            merged: merged.clone(),
            harness: cfg.harness.clone(),
            harness_thread_id: cfg.harness_thread_id.clone(),
            harness_home: cfg.harness_home.clone(),
            repo: manifest_repo,
        },
    )?;

    if lower_source.uses_fixture_seed() {
        seed_lower(&lower)?;
    }

    if is_overlay_mount(&merged)? {
        println!(
            "provision-overlay: session {} already mounted at {}",
            cfg.session,
            merged.display()
        );
        return Ok(());
    }

    reset_workdir(&work)?;

    let opts = format!(
        "lowerdir={},upperdir={},workdir={},metacopy=off",
        lower.display(),
        upper.display(),
        work.display()
    );
    let output = Command::new("mount")
        .args(["-t", "overlay", "overlay", "-o"])
        .arg(&opts)
        .arg(&merged)
        .output()
        .map_err(|e| format!("spawn mount: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "mount overlay failed (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    println!(
        "provision-overlay: mounted session {} upper={} lower={} work={} merged={}",
        cfg.session,
        upper.display(),
        lower.display(),
        work.display(),
        merged.display()
    );
    Ok(())
}

fn parse_args<I>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut session = None;
    let mut overlays_root = PathBuf::from("/var/lib/centaur/overlays");
    let mut merged_root = PathBuf::from("/run/centaur/merged");
    let mut lower = None;
    let mut harness = None;
    let mut harness_thread_id = String::new();
    let mut harness_home = String::new();
    let mut repo = String::new();
    let mut agent_uid = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let arg = arg
            .into_string()
            .map_err(|_| "arguments must be valid UTF-8".to_string())?;
        match arg.as_str() {
            "--session" => {
                session = Some(next_value(&mut iter, "--session")?);
            }
            "--overlays-root" => {
                overlays_root = PathBuf::from(next_value(&mut iter, "--overlays-root")?);
            }
            "--merged-root" => {
                merged_root = PathBuf::from(next_value(&mut iter, "--merged-root")?);
            }
            "--lower" => {
                lower = Some(PathBuf::from(next_value(&mut iter, "--lower")?));
            }
            "--harness" => {
                let value = next_value(&mut iter, "--harness")?;
                harness = normalize_harness(&value)?;
            }
            "--harness-thread-id" => {
                harness_thread_id = next_value(&mut iter, "--harness-thread-id")?;
            }
            "--harness-home" => {
                harness_home = next_value(&mut iter, "--harness-home")?;
            }
            "--repo" => {
                repo = next_value(&mut iter, "--repo")?;
            }
            "--agent-uid" => {
                let value = next_value(&mut iter, "--agent-uid")?;
                agent_uid = Some(value.parse::<u32>().map_err(|_| {
                    format!("--agent-uid requires an unsigned integer, got {value:?}")
                })?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {arg:?}")),
        }
    }

    Ok(Config {
        session: session.ok_or_else(|| "--session <ID> is required".to_string())?,
        overlays_root,
        merged_root,
        lower,
        harness,
        harness_thread_id,
        harness_home,
        repo,
        agent_uid,
    })
}

fn next_value(iter: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))?
        .into_string()
        .map_err(|_| format!("{flag} value must be valid UTF-8"))
}

fn print_help() {
    println!(
        "usage: provision-overlay --session <ID> [--overlays-root PATH] [--merged-root PATH] [--lower PATH] [--harness claude|codex|null] [--harness-thread-id ID] [--harness-home PATH] [--repo PATH] [--agent-uid UID]"
    );
}

fn validate_session(session: &str) -> Result<(), String> {
    if session.is_empty() {
        return Err("--session must not be empty".to_string());
    }
    let path = Path::new(session);
    if path.components().any(|c| {
        matches!(
            c,
            Component::RootDir | Component::CurDir | Component::ParentDir | Component::Prefix(_)
        )
    }) {
        return Err("--session must be a single path segment".to_string());
    }
    if session.contains('/') || session.contains('\0') {
        return Err("--session must be a single path segment".to_string());
    }
    Ok(())
}

fn seed_lower(lower: &Path) -> Result<(), String> {
    write_if_missing(&lower.join("seed.txt"), b"base seed\n")?;
    write_if_missing(&lower.join("delete-me.txt"), b"delete me\n")?;
    set_fixture_permissions(lower)
}

fn select_lower_source(
    repo: &str,
    lower: Option<&Path>,
    host_root: &Path,
    session: &str,
) -> Result<LowerSource, String> {
    let repo = repo.trim();
    if !repo.is_empty() {
        validate_repo_path_syntax(repo)?;
        return Ok(LowerSource {
            path: PathBuf::from(repo),
            kind: LowerKind::Repo,
        });
    }

    Ok(LowerSource {
        path: lower
            .map(Path::to_path_buf)
            .unwrap_or_else(|| host_root.join("overlay-lower").join(session)),
        kind: LowerKind::Fixture,
    })
}

fn resolve_lower_source(source: LowerSource) -> Result<LowerSource, String> {
    if source.kind != LowerKind::Repo {
        return Ok(source);
    }

    let metadata = std::fs::metadata(&source.path)
        .map_err(|e| format!("--repo {}: {e}", source.path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "--repo must be an existing directory, got {}",
            source.path.display()
        ));
    }
    let path = source
        .path
        .canonicalize()
        .map_err(|e| format!("canonicalize --repo {}: {e}", source.path.display()))?;
    Ok(LowerSource {
        path,
        kind: LowerKind::Repo,
    })
}

fn validate_repo_path_syntax(repo: &str) -> Result<(), String> {
    if repo.contains('\0') {
        return Err("--repo must not contain NUL bytes".to_string());
    }
    let path = Path::new(repo);
    let mut normal_components = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            Component::RootDir => {}
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err("--repo must not contain . or .. components".to_string());
            }
        }
    }
    if normal_components == 0 {
        return Err("--repo must name a directory".to_string());
    }
    Ok(())
}

fn write_if_missing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create lower parent {}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("write lower seed {}: {e}", path.display()))
}

#[cfg(unix)]
fn set_fixture_permissions(lower: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(lower, std::fs::Permissions::from_mode(0o777))
        .map_err(|e| format!("chmod lower {}: {e}", lower.display()))?;
    for entry in
        std::fs::read_dir(lower).map_err(|e| format!("read lower {}: {e}", lower.display()))?
    {
        let entry = entry.map_err(|e| format!("read lower entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777))
                .map_err(|e| format!("chmod lower dir {}: {e}", path.display()))?;
        } else {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))
                .map_err(|e| format!("chmod lower file {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_fixture_permissions(_lower: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode(dir: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", dir.display()))
}

#[cfg(not(unix))]
fn set_dir_mode(_dir: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn chown_dir_owner(dir: &Path, uid: u32) -> Result<(), String> {
    use std::os::unix::fs::chown;
    chown(dir, Some(uid), Some(uid)).map_err(|e| format!("chown {} to {uid}: {e}", dir.display()))
}

#[cfg(not(unix))]
fn chown_dir_owner(_dir: &Path, _uid: u32) -> Result<(), String> {
    Ok(())
}

fn reset_workdir(work: &Path) -> Result<(), String> {
    if work.exists() {
        std::fs::remove_dir_all(work)
            .map_err(|e| format!("reset workdir {}: {e}", work.display()))?;
    }
    std::fs::create_dir_all(work).map_err(|e| format!("create workdir {}: {e}", work.display()))
}

fn is_overlay_mount(merged: &Path) -> Result<bool, String> {
    let target = merged
        .canonicalize()
        .unwrap_or_else(|_| merged.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mountinfo = match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("read /proc/self/mountinfo: {e}")),
    };

    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        if fields.len() < 10 {
            continue;
        }
        let mount_point = unescape_mountinfo(fields[4]);
        let Some(sep) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        let Some(fs_type) = fields.get(sep + 1) else {
            continue;
        };
        if mount_point == target && *fs_type == "overlay" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn unescape_mountinfo(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Ok(octal) = u8::from_str_radix(&value[i + 1..i + 4], 8)
        {
            out.push(octal as char);
            i += 4;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_repo_uses_fixture_default_lower() {
        let selected =
            select_lower_source("", None, Path::new("/var/lib/centaur"), "sess-1").unwrap();

        assert_eq!(
            selected,
            LowerSource {
                path: PathBuf::from("/var/lib/centaur/overlay-lower/sess-1"),
                kind: LowerKind::Fixture,
            }
        );
        assert!(selected.uses_fixture_seed());
    }

    #[test]
    fn empty_repo_uses_fixture_lower_override() {
        let selected = select_lower_source(
            "  ",
            Some(Path::new("/tmp/custom-lower")),
            Path::new("/var/lib/centaur"),
            "sess-1",
        )
        .unwrap();

        assert_eq!(
            selected,
            LowerSource {
                path: PathBuf::from("/tmp/custom-lower"),
                kind: LowerKind::Fixture,
            }
        );
        assert!(selected.uses_fixture_seed());
    }

    #[test]
    fn repo_wins_over_fixture_lower_override() {
        let selected = select_lower_source(
            "/var/lib/centaur/repos/sess-1",
            Some(Path::new("/tmp/custom-lower")),
            Path::new("/var/lib/centaur"),
            "sess-1",
        )
        .unwrap();

        assert_eq!(
            selected,
            LowerSource {
                path: PathBuf::from("/var/lib/centaur/repos/sess-1"),
                kind: LowerKind::Repo,
            }
        );
        assert!(!selected.uses_fixture_seed());
    }

    #[test]
    fn repo_rejects_traversal_components() {
        let err = select_lower_source(
            "/var/lib/centaur/repos/../other",
            None,
            Path::new("/var/lib/centaur"),
            "sess-1",
        )
        .unwrap_err();

        assert!(err.contains(". or .."));
    }
}
