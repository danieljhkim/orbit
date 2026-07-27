use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_common::types::{Learning, LearningStatus, NotFoundKind, OrbitError};

use super::constants::LEARNING_SCHEMA_VERSION;
use super::doc::LearningFileDocument;
use super::layout::validate_learning_id;
use crate::file::yaml_doc::{read_yaml_with, serialize_yaml_with, write_yaml_atomic_with};

static LEARNING_CREATE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Read a learning YAML file at the given path. Returns a learning not-found error
/// when the file is missing on disk.
pub(super) fn read_learning_file(path: &Path) -> Result<Learning, OrbitError> {
    if !path.exists() {
        let id = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|n| n.to_str())
            .or_else(|| path.file_stem().and_then(|n| n.to_str()))
            .unwrap_or("<unknown>")
            .to_string();
        return Err(OrbitError::not_found(NotFoundKind::Learning, id));
    }
    let doc: LearningFileDocument = read_yaml_with(path, |path, err| {
        OrbitError::Store(format!("invalid learning file {}: {err}", path.display()))
    })?;
    Ok(doc.learning)
}

/// Write a learning record to disk at the given path. The directory is
/// created if missing; writes are atomic via the shared yaml-doc helper.
///
/// `expected_state` is asserted against `learning.status` to catch placement
/// bugs (e.g. writing a superseded record through an active-only call path).
pub(super) fn write_learning_file(
    path: &Path,
    learning: &Learning,
    expected_state: LearningStatus,
) -> Result<(), OrbitError> {
    validate_learning_id(&learning.id)?;
    if learning.status != expected_state {
        return Err(OrbitError::Store(format!(
            "learning '{}' status {:?} does not match destination state {:?}",
            learning.id, learning.status, expected_state
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    let doc = LearningFileDocument {
        schema_version: LEARNING_SCHEMA_VERSION,
        learning: learning.clone(),
    };
    write_yaml_atomic_with(path, &doc, |error| OrbitError::Store(error.to_string()))
}

/// Create a learning record at `path` without clobbering an existing body.
///
/// Returns `Ok(false)` when the target path already exists. The no-clobber
/// write stages the full YAML in the same directory, then hard-links it into
/// place so the final file appears atomically only if the destination is absent.
pub(super) fn create_learning_file_exclusive(
    path: &Path,
    learning: &Learning,
    expected_state: LearningStatus,
) -> Result<bool, OrbitError> {
    validate_learning_id(&learning.id)?;
    if learning.status != expected_state {
        return Err(OrbitError::Store(format!(
            "learning '{}' status {:?} does not match destination state {:?}",
            learning.id, learning.status, expected_state
        )));
    }
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Store(format!(
            "cannot determine learning file parent for '{}'",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let doc = LearningFileDocument {
        schema_version: LEARNING_SCHEMA_VERSION,
        learning: learning.clone(),
    };
    let yaml = serialize_yaml_with(&doc, |error| OrbitError::Store(error.to_string()))?;
    let temp_path = exclusive_temp_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    file.write_all(yaml.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    drop(file);

    match fs::hard_link(&temp_path, path) {
        Ok(()) => {
            fs::remove_file(&temp_path).map_err(|error| OrbitError::Io(error.to_string()))?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temp_path);
            Ok(false)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(OrbitError::Io(format!(
                "link {} to {}: {error}",
                temp_path.display(),
                path.display()
            )))
        }
    }
}

fn exclusive_temp_path(path: &Path) -> PathBuf {
    let counter = LEARNING_CREATE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("learning.yaml");
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ))
}
