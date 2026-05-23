use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

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

pub(crate) fn ensure_projection_entry_removable(
    workspace_orbit_dir: &Path,
    task_id: &str,
) -> Result<(), OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

pub(crate) fn remove_projection_entry(
    workspace_orbit_dir: &Path,
    task_id: &str,
) -> Result<bool, OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&projection_path).map_err(|err| OrbitError::Io(err.to_string()))?;
            Ok(true)
        }
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}
