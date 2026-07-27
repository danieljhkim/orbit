use std::fs;
use std::path::{Component, Path, PathBuf};

use orbit_common::types::OrbitError;

pub(crate) fn read_child_dirs(dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut child_dirs = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    child_dirs.sort();
    Ok(child_dirs)
}

pub(crate) fn validate_path_stem(stem: &str, kind: &str) -> Result<(), OrbitError> {
    if is_safe_path_stem(stem) {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "{kind} id must be a single path component without separators or traversal: {stem}"
    )))
}

fn is_safe_path_stem(stem: &str) -> bool {
    let mut components = Path::new(stem).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(part)), None) if part.to_str() == Some(stem)
    )
}
