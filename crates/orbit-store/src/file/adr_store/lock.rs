use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use orbit_common::types::OrbitError;

const LOCKS_DIR_NAME: &str = ".locks";

pub(super) fn acquire_adr_lock(root: &Path, id: &str) -> Result<File, OrbitError> {
    let path = adr_lock_path(root, id);
    acquire_lock(path, &format!("adr '{id}'"))
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

fn adr_lock_path(root: &Path, id: &str) -> PathBuf {
    root.join(LOCKS_DIR_NAME).join(format!("adr-{id}.lock"))
}
