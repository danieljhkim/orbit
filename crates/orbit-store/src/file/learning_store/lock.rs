use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use orbit_common::types::OrbitError;

use super::constants::{LEARNING_ALLOCATION_LOCK_FILE_NAME, LOCKS_DIR_NAME};

pub(super) fn acquire_learning_allocation_lock(root: &Path) -> Result<File, OrbitError> {
    let path = root
        .join(LOCKS_DIR_NAME)
        .join(LEARNING_ALLOCATION_LOCK_FILE_NAME);
    acquire_lock(path, "learning allocation")
}

pub(super) fn acquire_learning_lock(root: &Path, id: &str) -> Result<File, OrbitError> {
    let path = learning_lock_path(root, id);
    acquire_lock(path, &format!("learning '{id}'"))
}

fn acquire_lock(path: PathBuf, label: &str) -> Result<File, OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Store(format!(
            "cannot determine lock parent for '{}'",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|err| OrbitError::Io(err.to_string()))?;

    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|err| OrbitError::Io(err.to_string()))?;
    file.lock_exclusive().map_err(|err| {
        OrbitError::Store(format!(
            "failed to acquire {label} lock '{}': {err}",
            path.display()
        ))
    })?;
    Ok(file)
}

fn learning_lock_path(root: &Path, id: &str) -> PathBuf {
    root.join(LOCKS_DIR_NAME)
        .join(format!("learning-{id}.lock"))
}
