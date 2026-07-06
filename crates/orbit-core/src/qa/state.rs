//! qa-sweep host state [ORB-10039]: per-workspace last-validated watermarks
//! plus the single-flight pass lock.
//!
//! Both live in the **global** orbit dir (`~/.orbit/state/`), never in a
//! workspace `.orbit/` — the sweep is host-level scheduler machinery, and
//! per-repo state files would be one `orbit init`/task-mutation rewrite away
//! from being clobbered (see L-0041 / the hook-state watermark pattern).
//!
//! Watermark updates take an exclusive `fs2` lock on the state file and
//! re-read it under the lock, so concurrent writers (a manual run racing the
//! timer) merge instead of clobbering each other's workspaces.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

/// On-disk shape of `~/.orbit/state/qa-sweep.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct QaSweepState {
    /// Workspace name → watermark.
    #[serde(default)]
    pub workspaces: BTreeMap<String, QaWorkspaceWatermark>,
}

/// One workspace's last green validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QaWorkspaceWatermark {
    /// Commit sha of the last HEAD every configured (non-muted) check passed
    /// against. Failing sweeps never advance this.
    pub last_validated_sha: String,
    /// RFC 3339 timestamp of the validating sweep.
    pub validated_at: String,
    /// Ledger run id of the validating sweep, when one was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Path of the watermark state file under a global orbit root.
pub fn state_path(global_root: &Path) -> PathBuf {
    global_root.join("state").join("qa-sweep.json")
}

/// Path of the single-flight pass lock under a global orbit root.
pub(crate) fn lock_path(global_root: &Path) -> PathBuf {
    global_root.join("state").join("qa-sweep.lock")
}

/// Read the current state (missing or unparsable file → empty state; the
/// sweep then treats every workspace as never-validated, which only means it
/// re-validates the current HEAD — safe in both directions).
pub fn load_state(path: &Path) -> QaSweepState {
    fs::read_to_string(path)
        .ok()
        .map(|raw| parse_state(&raw))
        .unwrap_or_default()
}

pub(crate) fn parse_state(raw: &str) -> QaSweepState {
    serde_json::from_str(raw.trim()).unwrap_or_default()
}

/// Advance one workspace's watermark under an exclusive file lock,
/// read-modify-writing the file so other workspaces' entries survive.
pub(crate) fn advance_watermark(
    path: &Path,
    workspace: &str,
    watermark: QaWorkspaceWatermark,
) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Io(format!(
                "create qa-sweep state dir {}: {error}",
                parent.display()
            ))
        })?;
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            OrbitError::Io(format!(
                "open qa-sweep state file {}: {error}",
                path.display()
            ))
        })?;
    file.lock_exclusive().map_err(|error| {
        OrbitError::Io(format!(
            "lock qa-sweep state file {}: {error}",
            path.display()
        ))
    })?;

    let result = write_locked(&mut file, path, workspace, watermark);
    let unlock = fs2::FileExt::unlock(&file).map_err(|error| {
        OrbitError::Io(format!(
            "unlock qa-sweep state file {}: {error}",
            path.display()
        ))
    });
    result.and(unlock)
}

fn write_locked(
    file: &mut File,
    path: &Path,
    workspace: &str,
    watermark: QaWorkspaceWatermark,
) -> Result<(), OrbitError> {
    let mut raw = String::new();
    file.read_to_string(&mut raw)
        .map_err(|error| OrbitError::Io(format!("read {}: {error}", path.display())))?;
    let mut state = parse_state(&raw);
    state.workspaces.insert(workspace.to_string(), watermark);

    let encoded = serde_json::to_string_pretty(&state)
        .map_err(|error| OrbitError::Io(format!("encode qa-sweep state: {error}")))?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.set_len(0))
        .and_then(|_| file.write_all(encoded.as_bytes()))
        .map_err(|error| OrbitError::Io(format!("write {}: {error}", path.display())))
}

/// Held for the duration of one sweep pass; the OS releases the lock on drop
/// or process death, so a crashed sweep never wedges the next one.
pub(crate) struct PassLock {
    _file: File,
}

/// Try to become the single in-flight sweep on this host. `Ok(None)` means
/// another pass holds the lock.
pub(crate) fn try_acquire_pass_lock(global_root: &Path) -> Result<Option<PassLock>, OrbitError> {
    let path = lock_path(global_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Io(format!(
                "create qa-sweep lock dir {}: {error}",
                parent.display()
            ))
        })?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| {
            OrbitError::Io(format!("open qa-sweep lock {}: {error}", path.display()))
        })?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(PassLock { _file: file })),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(OrbitError::Io(format!(
            "lock qa-sweep lock {}: {error}",
            path.display()
        ))),
    }
}
