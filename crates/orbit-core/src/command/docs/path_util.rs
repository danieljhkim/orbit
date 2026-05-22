use std::path::{Component, Path, PathBuf};

use orbit_common::types::OrbitError;

pub(super) fn repo_relative_path(repo_root: &Path, path: &Path) -> Result<PathBuf, OrbitError> {
    if let Ok(relative) = path.strip_prefix(repo_root) {
        return Ok(relative.to_path_buf());
    }
    let canonical_repo = repo_root.canonicalize().map_err(|error| {
        OrbitError::Io(format!("canonicalize {}: {error}", repo_root.display()))
    })?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| OrbitError::Io(format!("canonicalize {}: {error}", path.display())))?;
    canonical_path
        .strip_prefix(canonical_repo)
        .map(Path::to_path_buf)
        .map_err(|_| {
            OrbitError::InvalidInput(format!(
                "path is outside workspace root: {}",
                path.display()
            ))
        })
}

pub(super) fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn component_str(component: Component<'_>) -> Option<&str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}
