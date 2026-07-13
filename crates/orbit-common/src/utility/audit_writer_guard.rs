//! Workspace-scoped advisory guard shared by the audit writer paths and the
//! audit garbage collector.
//!
//! ADR-0220 established the host-global GC lock that serializes `gc apply`
//! against other GC processes. That lock does **not** serialize GC against a
//! worker *writing* audit evidence, so the window between the audit collector's
//! final mark/fingerprint revalidation and its envelope/blob `remove_file` was
//! not atomic against a concurrent writer (ORB-10186). A writer could publish a
//! retained v2 envelope (or append a loop JSONL partition) referencing a blob
//! after GC re-marked it unreachable and before GC unlinked it, leaving a
//! retained envelope pointing at a swept blob — or lose an append that GC
//! deleted between the fingerprint check and the write.
//!
//! This guard closes that window: one advisory file lock per workspace audit
//! root that **both** competing paths acquire —
//!
//! - every audit writer path holds it across its publication —
//!   workspace v2 event publication, loop event/JSONL append, and
//!   content-addressed blob publication — and
//! - the audit collector holds it continuously across
//!   `[final mark/fingerprint revalidation .. envelope/blob deletion]`.
//!
//! Lock ordering is fixed: **GC host lock → audit writer guard → filesystem
//! mutation**. The guard is a filesystem advisory lock, never the global SQLite
//! write lock, so no unrelated database lock is held across a blob/JSONL
//! deletion. Both sides derive the lock path identically from the workspace
//! `state/audit` dir so they rendezvous on the same inode. The lock file lives
//! directly at the audit root (a dotfile) so it is never mistaken for a v2
//! event, a JSONL partition, a blob, or protected evidence by the collector's
//! scans.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;

use crate::types::OrbitError;

/// Bounded wait before a contender fails closed rather than blocking a writer
/// (or the collector) indefinitely. Contention is per-workspace and only ever
/// occurs at the instant one writer publishes while GC is deleting a candidate
/// in the same workspace, so this bound is a safety valve, not a hot-path cost.
const GUARD_WAIT: Duration = Duration::from_secs(5);
const GUARD_POLL: Duration = Duration::from_millis(25);

/// File name of the per-workspace audit writer/GC lock, held directly at the
/// audit root.
const GUARD_FILE_NAME: &str = ".gc-writer.lock";

/// RAII holder for an acquired audit writer/GC guard. Dropping it releases the
/// advisory lock; the open descriptor is kept alive for the lock's lifetime.
#[derive(Debug)]
pub struct AuditWriterGuard {
    file: File,
}

impl Drop for AuditWriterGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// The advisory lock file, derived identically by the writer paths and the
/// collector from the workspace `state/audit` dir.
pub fn guard_path(audit_root: &Path) -> PathBuf {
    audit_root.join(GUARD_FILE_NAME)
}

fn open_guard_file(audit_root: &Path) -> Result<File, OrbitError> {
    fs::create_dir_all(audit_root)?;
    let path = guard_path(audit_root);
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?)
}

/// Blocking acquire with a bounded wait. Returns the RAII guard, or a timeout
/// error so callers fail closed instead of blocking forever. The collector
/// treats the error as "do not delete"; a writer surfaces it as a non-fatal
/// audit-write failure rather than publishing outside the guard.
pub fn acquire(audit_root: &Path) -> Result<AuditWriterGuard, OrbitError> {
    let file = open_guard_file(audit_root)?;
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(AuditWriterGuard { file }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= GUARD_WAIT {
                    return Err(OrbitError::Execution(format!(
                        "timed out waiting for audit writer/GC guard `{}`",
                        guard_path(audit_root).display()
                    )));
                }
                thread::sleep(GUARD_POLL);
            }
            Err(error) => return Err(error.into()),
        }
    }
}
