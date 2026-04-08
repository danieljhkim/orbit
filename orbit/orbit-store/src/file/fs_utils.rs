use std::fs;
use std::path::Path;

use chrono::Utc;
use orbit_types::OrbitError;

/// Atomically writes `content` to `path` by writing to a timestamped `.tmp` sibling,
/// then renaming. Cleans up the temp file on rename failure.
pub(crate) fn write_atomic(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Io(format!("cannot determine parent for '{}'", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| OrbitError::Io(format!("path '{}' has no file name", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.tmp.{nanos}"));

    fs::write(&tmp_path, content).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
}
