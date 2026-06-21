//! centaur-node-syncd — the per-node sync daemon (Track C4). Runs the two sweeps
//! per session: scan the overlay `upper` → capture changes to Atrium; poll the
//! gap-free change-feed → adopt remote advances by writing through `merged` at a
//! quiesce point. Egress-only (x-api-key to Atrium's internal node endpoints).
//!
//! Env:
//!   ATRIUM_BASE_URL, ATRIUM_CAPTURE_API_KEY  — the Atrium endpoint + key
//!   NODE_SYNC_SESSION                         — the session id this run drives
//!   NODE_SYNC_UPPER                           — the overlay upperdir to scan
//!   NODE_SYNC_MERGED                          — the merged mount to write through
//!   NODE_SYNC_INTERVAL_SECS (default 2)       — scan cadence
//! Flags: --once (one capture + one inbound sweep, then exit — for the e2e).

#[cfg(target_os = "linux")]
fn main() {
    use centaur_node_sync::echo::EchoGuard;
    use centaur_node_sync::fs_linux;
    use centaur_node_sync::http_client::HttpAtriumClient;
    use centaur_node_sync::quiesce::{apply_quiesced_writes, LeaseGate};
    use centaur_node_sync::runtime::{capture_sweep, inbound_sweep, AtriumClient, UpperReader};
    use centaur_node_sync::adopt::LocalState;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    let once = std::env::args().any(|a| a == "--once");
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let base_url = env("ATRIUM_BASE_URL");
    let api_key = env("ATRIUM_CAPTURE_API_KEY");
    let session = env("NODE_SYNC_SESSION");
    let upper = PathBuf::from(env("NODE_SYNC_UPPER"));
    let merged = PathBuf::from(env("NODE_SYNC_MERGED"));
    let interval = env("NODE_SYNC_INTERVAL_SECS").parse::<u64>().unwrap_or(2);
    if base_url.is_empty() || session.is_empty() || upper.as_os_str().is_empty() {
        eprintln!("missing ATRIUM_BASE_URL / NODE_SYNC_SESSION / NODE_SYNC_UPPER");
        std::process::exit(2);
    }

    struct HardenedReader {
        upper: PathBuf,
    }
    impl UpperReader for HardenedReader {
        fn read(&self, rel: &PathBuf) -> Option<Vec<u8>> {
            fs_linux::read_file_safe(&self.upper, rel, 3).ok()
        }
    }

    // Write reconciled bytes THROUGH `merged` (the only legal external write path):
    // atomic temp+rename + agent ownership (uid 1001, 0664). NEVER poke upper/lower.
    fn write_through_merged(merged: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
        use std::os::unix::fs::chown;
        let dst = merged.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = dst.with_extension("nodesync.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        let _ = chown(&tmp, Some(1001), Some(1001));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o664));
        }
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
        Ok(())
    }

    let mut client = HttpAtriumClient::new(base_url, api_key, &session);
    let mut echo = EchoGuard::new();
    let lease = LeaseGate::new(); // harness flips this in prod; empty = all quiesced
    let base_seqs: HashMap<String, u64> = HashMap::new(); // hydration manifest seed
    let locals: HashMap<String, LocalState> = HashMap::new();
    let mut cursor = "0.0".to_string();

    loop {
        // OUTBOUND — capture sweep
        match fs_linux::read_upper_entries(&upper) {
            Ok(entries) => {
                let reader = HardenedReader { upper: upper.clone() };
                let out = capture_sweep(&entries, &base_seqs, &reader, &mut echo, &mut client);
                println!(
                    "capture: {} upserts, {} deletes, {} echo-skipped, {} errors",
                    out.captured.len(),
                    out.deleted.len(),
                    out.skipped_echo.len(),
                    out.errors.len()
                );
                for (p, e) in &out.errors {
                    eprintln!("  capture error {p}: {e}");
                }
            }
            Err(e) => eprintln!("scan {}: {e}", upper.display()),
        }

        // INBOUND — poll feed → adopt
        match client.poll_changes(&cursor) {
            Ok((changes, next)) => {
                cursor = next;
                if !changes.is_empty() {
                    let plan = inbound_sweep(&changes, &locals, &mut echo, &mut client);
                    let (written, deferred) =
                        apply_quiesced_writes(plan.to_write, &lease, |rel, bytes| {
                            write_through_merged(&merged, rel, bytes)
                        });
                    println!(
                        "inbound: {} adopted, {} deferred, {} reconcile, {} conflicts",
                        written.len(),
                        deferred.len(),
                        plan.to_reconcile.len(),
                        plan.conflicts.len()
                    );
                }
            }
            Err(e) => eprintln!("poll: {e}"),
        }

        if once {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("centaur-node-syncd runs on linux nodes only");
    std::process::exit(1);
}
