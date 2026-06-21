//! WIP capture for uncommitted repo work (§5A, DECIDED: pure-read patch-artifact,
//! NOT a git ref). Repo files are excluded from the artifact ledger (branch-
//! incoherent), so git only makes work durable at commit — uncommitted edits are
//! lost on crash/destroy. The node captures them as a **read-only** `git diff HEAD`
//! + untracked-file snapshot stored as a ledger blob, creating ZERO git objects or
//! refs the agents that share the repo could trip over. Recovery = re-clone at
//! `base_head_sha`, `git apply` the diff, drop in the untracked files.
//!
//! Trade-off (accepted): a recovery point, not a faithful clone — in-progress
//! rebase/merge, staged-vs-unstaged, and submodule state aren't captured.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct WipPatch {
    pub base_head_sha: String,
    /// `git diff HEAD` (tracked changes, binary-safe).
    pub diff: String,
    /// Untracked, .gitignore-respected files: (relpath, bytes).
    pub untracked: Vec<(String, Vec<u8>)>,
}

impl WipPatch {
    pub fn is_empty(&self) -> bool {
        self.diff.is_empty() && self.untracked.is_empty()
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("spawn git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

/// Capture the repo's uncommitted working state. PURE READ — runs only `rev-parse`,
/// `diff`, and `ls-files`; never `add`/`commit`/`write`, so it touches no
/// ref/index/object the fleet can see.
pub fn capture_wip(repo: &Path) -> Result<WipPatch, String> {
    let base_head_sha = String::from_utf8_lossy(&git(repo, &["rev-parse", "HEAD"])?)
        .trim()
        .to_string();
    let diff = String::from_utf8_lossy(&git(repo, &["diff", "HEAD"])?).into_owned();

    // Untracked, gitignore-respected. -z = NUL-separated (safe for odd names).
    let raw = git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut untracked = Vec::new();
    for name in raw.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let rel = String::from_utf8_lossy(name).into_owned();
        if let Ok(bytes) = std::fs::read(repo.join(&rel)) {
            untracked.push((rel, bytes));
        }
    }
    Ok(WipPatch {
        base_head_sha,
        diff,
        untracked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn sh(repo: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    // A unique temp dir without Date/rand (sandbox bans them): use the test's
    // module path + the repo-relative pid via std::process.
    fn tmp_repo(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("wip-it-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn captures_tracked_diff_and_untracked_without_writing_refs() {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git absent; skipping");
            return;
        }
        let repo = tmp_repo("a");
        sh(&repo, &["init", "-q"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        fs::write(repo.join("tracked.txt"), "v1\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);

        // uncommitted work: modify tracked + add untracked
        fs::write(repo.join("tracked.txt"), "v1\nWIP edit\n").unwrap();
        fs::write(repo.join("scratch.txt"), "brand new\n").unwrap();

        let refs_before = git(&repo, &["show-ref"]).unwrap_or_default();
        let wip = capture_wip(&repo).unwrap();
        let refs_after = git(&repo, &["show-ref"]).unwrap_or_default();

        assert_eq!(wip.base_head_sha.len(), 40);
        assert!(
            wip.diff.contains("WIP edit"),
            "diff carries the tracked change"
        );
        assert!(
            wip.untracked
                .iter()
                .any(|(p, b)| p == "scratch.txt" && b == b"brand new\n"),
            "untracked file captured with bytes",
        );
        // the key invariant: capture created NO refs/objects the fleet can see.
        assert_eq!(
            refs_before, refs_after,
            "capture_wip must not write any git ref"
        );

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn clean_repo_yields_empty_patch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let repo = tmp_repo("b");
        sh(&repo, &["init", "-q"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        fs::write(repo.join("f.txt"), "x\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);

        let wip = capture_wip(&repo).unwrap();
        assert!(wip.is_empty(), "no uncommitted work → empty patch");
        let _ = fs::remove_dir_all(&repo);
    }
}
