use std::path::{Component, Path, PathBuf};

use orbit_common::OrbitError;

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

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
