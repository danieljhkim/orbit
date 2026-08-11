use std::path::{Component, Path};

use orbit_common::types::OrbitError;

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
