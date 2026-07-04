use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

use crate::file_lock::{FileLockGuard, acquire_exclusive};

const LOCKS_DIR_NAME: &str = ".locks";

pub(super) fn acquire_adr_lock(root: &Path, id: &str) -> Result<FileLockGuard, OrbitError> {
    let path = adr_lock_path(root, id);
    // [ORB-00412] Bounded acquisition with holder diagnostics.
    acquire_exclusive(&path, &format!("adr '{id}'"))
}

fn adr_lock_path(root: &Path, id: &str) -> PathBuf {
    root.join(LOCKS_DIR_NAME).join(format!("adr-{id}.lock"))
}
