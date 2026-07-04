use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

use super::constants::{LEARNING_ALLOCATION_LOCK_FILE_NAME, LOCKS_DIR_NAME};
use crate::file_lock::{FileLockGuard, acquire_exclusive};

pub(super) fn acquire_learning_allocation_lock(root: &Path) -> Result<FileLockGuard, OrbitError> {
    let path = root
        .join(LOCKS_DIR_NAME)
        .join(LEARNING_ALLOCATION_LOCK_FILE_NAME);
    // [ORB-00412] Bounded acquisition with holder diagnostics.
    acquire_exclusive(&path, "learning allocation")
}

pub(super) fn acquire_learning_lock(root: &Path, id: &str) -> Result<FileLockGuard, OrbitError> {
    let path = learning_lock_path(root, id);
    acquire_exclusive(&path, &format!("learning '{id}'"))
}

fn learning_lock_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!(".{id}.lock"))
}
