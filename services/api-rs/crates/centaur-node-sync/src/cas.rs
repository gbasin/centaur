//! Node-local CAS cache + reflink materialization (Phase 5 scale layer, §8B #18).
//!
//! Hydrating a lower is a CAS checkout: per artifact path, materialize one blob.
//! At scale we don't re-download or byte-copy per pod — blobs are content-addressed
//! in a node-local cache (`/var/lib/centaur/cas/<sha>`, immutable) and **reflinked**
//! (FICLONE, copy-on-write) into each pod's lower tree → near-zero time + disk, free
//! dedup across pods. Reflink requires XFS/btrfs (#18); on other FS the clone
//! ioctl returns EOPNOTSUPP and we fall back to a full copy (safe, just not shared —
//! NEVER a hardlink, which would share the inode and let one pod corrupt the CAS).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

/// One hydration entry resolved to its content hash.
#[derive(Debug, Clone)]
pub struct CasHydrateEntry {
    pub path: String,
    pub seq: u64,
    pub sha: String,
}

pub struct MaterializeOutcome {
    pub base_seqs: HashMap<String, u64>,
    pub reflinked: u64,
    pub copied: u64,
    pub fetched: u64,
    pub errors: Vec<(String, String)>,
}

/// Reflink `src` → `dst` (FICLONE), falling back to a full copy when the FS
/// doesn't support it. Never hardlinks.
pub fn reflink_or_copy(src: &Path, dst: &Path) -> io::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        match try_ficlone(src, dst) {
            Ok(()) => return Ok(true),
            Err(_) => { /* fall through to copy */ }
        }
    }
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    fs::copy(src, dst)?;
    Ok(false)
}

#[cfg(target_os = "linux")]
fn try_ficlone(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // FICLONE = _IOW(0x94, 9, int) == 0x40049409 on all arches.
    const FICLONE: libc::c_ulong = 0x40049409;
    let s = fs::File::open(src)?;
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    let d = fs::OpenOptions::new().write(true).create(true).truncate(true).open(dst)?;
    let ret = unsafe { libc::ioctl(d.as_raw_fd(), FICLONE, s.as_raw_fd()) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Probe whether the directory's filesystem supports reflink (FICLONE). Writes two
/// tiny temp files, clones one to the other, cleans up. Used at node admission to
/// refuse a non-reflink node (#18) — or to decide copy-fallback up front.
pub fn probe_reflink(dir: &Path) -> bool {
    let a = dir.join(".reflink-probe-src");
    let b = dir.join(".reflink-probe-dst");
    let _ = fs::write(&a, b"probe");
    let ok = reflink_or_copy(&a, &b).map(|reflinked| reflinked).unwrap_or(false);
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
    ok
}

fn cas_path(cas_dir: &Path, sha: &str) -> std::path::PathBuf {
    // shard by first 2 hex chars (matches the Atrium cas/<2>/<sha> layout)
    cas_dir.join(&sha[..sha.len().min(2)]).join(sha)
}

/// Ensure a blob is in the node-local CAS (atomic temp+rename; immutable 0444).
/// Returns true if it had to be written (a cache miss → the caller fetched it).
pub fn ensure_cas_blob(
    cas_dir: &Path,
    sha: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<bool, String> {
    let target = cas_path(cas_dir, sha);
    if target.exists() {
        return Ok(false);
    }
    let bytes = fetch()?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = target.with_extension("tmp");
    fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o444));
    }
    fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Materialize the artifact lower from the CAS: for each entry, ensure its blob is
/// cached (fetch on miss), then reflink it into `lower_root/<path>`. Returns the
/// per-path base_seqs (the sync-state seed) + reflink/copy/fetch counters.
pub fn materialize_cached(
    entries: &[CasHydrateEntry],
    cas_dir: &Path,
    lower_root: &Path,
    mut fetch: impl FnMut(&str, u64) -> Result<Vec<u8>, String>,
) -> MaterializeOutcome {
    let mut out = MaterializeOutcome {
        base_seqs: HashMap::new(),
        reflinked: 0,
        copied: 0,
        fetched: 0,
        errors: vec![],
    };
    for e in entries {
        let r: Result<(), String> = (|| {
            let miss = ensure_cas_blob(cas_dir, &e.sha, || fetch(&e.path, e.seq))?;
            if miss {
                out.fetched += 1;
            }
            let dst = lower_root.join(&e.path);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            let reflinked = reflink_or_copy(&cas_path(cas_dir, &e.sha), &dst).map_err(|err| err.to_string())?;
            if reflinked {
                out.reflinked += 1;
            } else {
                out.copied += 1;
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                out.base_seqs.insert(e.path.clone(), e.seq);
            }
            Err(err) => out.errors.push((e.path.clone(), err)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cas-it-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reflink_or_copy_round_trips_bytes() {
        let d = tmp("rl");
        let src = d.join("src");
        let dst = d.join("dst");
        fs::write(&src, b"hello cas").unwrap();
        reflink_or_copy(&src, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"hello cas");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn materialize_caches_then_reuses_blob_and_lays_out_tree() {
        let root = tmp("mat");
        let cas = root.join("cas");
        let lower = root.join("lower");
        fs::create_dir_all(&cas).unwrap();
        let entries = vec![
            CasHydrateEntry { path: "proj-x/a.md".into(), seq: 5, sha: "aa11".into() },
            CasHydrateEntry { path: "proj-x/b.md".into(), seq: 6, sha: "bb22".into() },
            // same blob as a.md (dedup) under a different path
            CasHydrateEntry { path: "shared/copy.md".into(), seq: 7, sha: "aa11".into() },
        ];
        let mut fetched: Vec<String> = vec![];
        let out = materialize_cached(&entries, &cas, &lower, |path, _seq| {
            fetched.push(path.to_string());
            Ok(format!("bytes for {path}").into_bytes())
        });
        // a.md + b.md fetched; copy.md reused the cached aa11 blob (no 3rd fetch).
        assert_eq!(out.fetched, 2, "shared sha fetched once");
        assert_eq!(out.base_seqs.get("proj-x/a.md"), Some(&5));
        assert!(lower.join("proj-x/a.md").exists());
        assert!(lower.join("shared/copy.md").exists());
        // copy.md materialized from the SAME cached blob as a.md.
        assert_eq!(
            fs::read(lower.join("shared/copy.md")).unwrap(),
            fs::read(lower.join("proj-x/a.md")).unwrap(),
        );
        assert!(out.errors.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_reflink_runs_without_panicking() {
        let d = tmp("probe");
        let _ = probe_reflink(&d); // true on xfs/btrfs, false (copy) elsewhere — both fine
        let _ = fs::remove_dir_all(&d);
    }
}
