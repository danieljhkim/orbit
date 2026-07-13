//! Per-run advisory claim/reclaim guard shared by the run claim/start path and
//! the managed-worktree collector.
//!
//! ADR-0220 established the host-global GC lock that serializes `gc apply`
//! against other GC processes. That lock does **not** serialize GC against a
//! worker claiming or reclaiming a run, so the window between the collector's
//! final ownership revalidation and `git worktree remove` was not atomic against
//! a concurrent claim (ORB-10182). This guard closes that window: one advisory
//! file lock keyed by run id that **both** competing paths acquire —
//!
//! - the run claim/start path (`mark_run_running`, `claim_pending_run_owner`,
//!   `take_over_running_run`) holds it across the ownership state transition, and
//! - the worktree collector holds it continuously across
//!   `[final revalidation .. git worktree remove]`.
//!
//! Lock ordering is fixed: **GC host lock → per-run guard → filesystem
//! mutation**. The guard is a filesystem advisory lock, never the global SQLite
//! write lock, so no unrelated database lock is held across `git worktree
//! remove`. Both sides derive the lock path identically from the workspace
//! `state` dir so they rendezvous on the same inode; a run id that is empty or
//! contains a path separator is rejected rather than allowed to escape the
//! guard directory.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;
use orbit_common::types::OrbitError;

/// Bounded wait before a contender fails closed rather than blocking a worker
/// (or the collector) indefinitely. Contention is per-run and only ever occurs
/// at the instant one run transitions ownership while its own worktree is being
/// collected, so this bound is a safety valve, not a hot-path cost.
const GUARD_WAIT: Duration = Duration::from_secs(5);
const GUARD_POLL: Duration = Duration::from_millis(25);

/// RAII holder for an acquired per-run guard. Dropping it releases the advisory
/// lock; the open descriptor is kept alive for the lock's lifetime.
#[derive(Debug)]
pub(crate) struct RunClaimGuard {
    file: File,
}

impl Drop for RunClaimGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Directory holding one lock file per run id, derived identically by the claim
/// path and the collector from the workspace `state` dir.
fn guard_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("run-guards")
}

fn guard_path(state_dir: &Path, run_id: &str) -> Result<PathBuf, OrbitError> {
    if run_id.is_empty() || run_id.contains('/') || run_id.contains('\\') || run_id.contains("..") {
        return Err(OrbitError::InvalidInput(format!(
            "refusing to derive a run-claim guard path for suspicious run id `{run_id}`"
        )));
    }
    Ok(guard_dir(state_dir).join(format!("{run_id}.lock")))
}

fn open_guard_file(state_dir: &Path, run_id: &str) -> Result<File, OrbitError> {
    let path = guard_path(state_dir, run_id)?;
    fs::create_dir_all(guard_dir(state_dir))?;
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?)
}

/// Blocking acquire with a bounded wait. Returns the RAII guard, or a timeout
/// error so callers fail closed instead of blocking forever. The collector
/// treats the error as "do not remove"; the claim path surfaces it as a failed
/// start that reconciliation retries.
pub(crate) fn acquire(state_dir: &Path, run_id: &str) -> Result<RunClaimGuard, OrbitError> {
    let file = open_guard_file(state_dir, run_id)?;
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(RunClaimGuard { file }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= GUARD_WAIT {
                    return Err(OrbitError::Execution(format!(
                        "timed out waiting for per-run claim guard `{run_id}`"
                    )));
                }
                thread::sleep(GUARD_POLL);
            }
            Err(error) => return Err(error.into()),
        }
    }
}
