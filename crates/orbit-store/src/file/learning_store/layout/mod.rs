use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;
use serde_json::Value;

#[cfg(test)]
use super::constants::LEARNING_DOC_FILE_EXT;
use super::constants::{LEARNING_COMMENTS_FILE_NAME, LEARNING_DOC_FILE_NAME};

pub(super) fn learning_dir_path(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

pub(super) fn learning_doc_path(root: &Path, id: &str) -> PathBuf {
    learning_dir_path(root, id).join(LEARNING_DOC_FILE_NAME)
}

pub(super) fn votes_jsonl_path(root: &Path, id: &str) -> PathBuf {
    learning_dir_path(root, id).join("votes.jsonl")
}

pub(super) fn comments_jsonl_path(root: &Path, id: &str) -> PathBuf {
    learning_dir_path(root, id).join(LEARNING_COMMENTS_FILE_NAME)
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

/// Allocate the next sequential learning comment id of the form
/// `C<YYYYMMDD>-<NNNN>`.
///
/// **Caller contract**: hold the learning allocation lock for the scan and
/// subsequent append so concurrent adders cannot choose the same id.
pub(super) fn next_learning_comment_id(
    root: &Path,
    now: DateTime<Utc>,
) -> Result<String, OrbitError> {
    let date = now.format("%Y%m%d").to_string();
    let prefix = format!("C{date}-");
    let mut max_suffix: u32 = 0;

    if root.exists() {
        for entry in fs::read_dir(root).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            if !file_type.is_dir() {
                continue;
            }
            let Some(learning_id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_learning_id(&learning_id).is_err() {
                continue;
            }
            let path = comments_jsonl_path(root, &learning_id);
            let Ok(raw) = fs::read_to_string(path) else {
                continue;
            };
            for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(tail) = id.strip_prefix(&prefix) else {
                    continue;
                };
                if let Ok(n) = tail.parse::<u32>() {
                    max_suffix = max_suffix.max(n);
                }
            }
        }
    }

    let next = max_suffix
        .checked_add(1)
        .ok_or_else(|| OrbitError::Execution("learning comment id counter overflow".to_string()))?;
    Ok(format!("C{date}-{next}"))
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

pub(super) fn validate_learning_comment_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_learning_comment_id(id) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "learning comment id must match C<YYYYMMDD>-<digits>: {id}"
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

fn is_valid_learning_comment_id(id: &str) -> bool {
    let Some(raw) = id.strip_prefix('C') else {
        return false;
    };
    if raw.len() < 10 {
        return false;
    }
    let Some(date) = raw.get(0..8) else {
        return false;
    };
    if !date.as_bytes().iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Some(month) = date.get(4..6) else {
        return false;
    };
    if !matches!(
        month,
        "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
    ) {
        return false;
    }
    let Some(tail) = raw.get(8..).and_then(|value| value.strip_prefix('-')) else {
        return false;
    };
    !tail.is_empty() && tail.as_bytes().iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
#[cfg(test)]
mod tests;
