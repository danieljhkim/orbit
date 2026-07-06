#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;

#[cfg(test)]
use super::constants::LEARNING_DOC_FILE_EXT;
use super::constants::LEARNING_DOC_FILE_NAME;

pub(super) fn learning_dir_path(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

pub(super) fn learning_doc_path(root: &Path, id: &str) -> PathBuf {
    learning_dir_path(root, id).join(LEARNING_DOC_FILE_NAME)
}

/// Locate the YAML path of a learning by id, or `None` if missing.
pub(super) fn locate_learning(root: &Path, id: &str) -> Result<Option<PathBuf>, OrbitError> {
    validate_learning_id(id)?;
    let path = learning_doc_path(root, id);
    if path.is_file() {
        return Ok(Some(path));
    }
    Ok(None)
}

/// Allocate the next sequential learning id of the form `L-NNNN`.
///
/// `<NNNN>` is monotonically increasing across every per-entity learning
/// directory. Runtime-backed stores use the SQLite id allocator; this scan
/// helper remains for layout-focused tests and legacy fallback checks.
///
/// **Caller contract**: must hold an allocation lock (see
/// [`super::lock::acquire_learning_allocation_lock`]) for the duration of
/// the scan and the subsequent file creation, so the scan-then-allocate
/// window remains serialized across concurrent writers.
#[cfg(test)]
pub(super) fn next_learning_id(root: &Path, _now: DateTime<Utc>) -> Result<String, OrbitError> {
    let mut max_suffix: u32 = 0;

    if root.exists() {
        for entry in fs::read_dir(root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(id) = learning_id_from_layout_entry(&name, file_type.is_dir()) else {
                continue;
            };
            if file_type.is_dir() && !learning_doc_path(root, &id).is_file() {
                continue;
            }
            if let Some(n) = parse_learning_sequence(&id) {
                max_suffix = max_suffix.max(n);
            }
        }
    }

    let next = max_suffix
        .checked_add(1)
        .ok_or_else(|| OrbitError::Execution("learning id counter overflow".to_string()))?;
    let width = next.to_string().len().max(4);
    Ok(format!("L-{next:0width$}"))
}

#[cfg(test)]
fn learning_id_from_layout_entry(name: &str, is_dir: bool) -> Option<String> {
    if is_dir {
        return is_valid_learning_id(name).then(|| name.to_string());
    }
    let stem = name.strip_suffix(&format!(".{LEARNING_DOC_FILE_EXT}"))?;
    is_valid_learning_id(stem).then(|| stem.to_string())
}

/// Validate that `id` is shaped as `L-NNNN` and free of path
/// traversal characters.
pub(super) fn validate_learning_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_learning_id(id) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "learning id must match L-NNNN: {id}"
    )))
}

fn is_valid_learning_id(id: &str) -> bool {
    parse_learning_sequence(id).is_some()
}

fn parse_learning_sequence(id: &str) -> Option<u32> {
    let suffix = id.strip_prefix("L-")?;
    if suffix.len() < 4 || !suffix.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

#[cfg(test)]
mod tests;
