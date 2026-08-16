use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::OrbitError;

pub(crate) fn task_bundle_lock_sentinel_path(bundle_dir: &Path) -> Result<PathBuf, OrbitError> {
    let file_name = bundle_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            OrbitError::Store(format!(
                "task bundle path {} has no file name",
                bundle_dir.display()
            ))
        })?;
    Ok(bundle_dir.with_file_name(format!(".{file_name}.lock")))
}

pub(crate) fn remove_task_bundle_lock_sentinel(lock_path: &Path) -> Result<(), OrbitError> {
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}
