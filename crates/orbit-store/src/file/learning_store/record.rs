use std::fs;
use std::path::Path;

use orbit_common::types::{Learning, LearningStatus, NotFoundKind, OrbitError};

use super::constants::LEARNING_SCHEMA_VERSION;
use super::doc::{LearningFileDocument, serialize_learning_doc_yaml};
use super::layout::validate_learning_id;
use crate::file::yaml_doc::{read_yaml_with, write_yaml_atomic_with};

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
    write_yaml_atomic_with(path, &doc, serialize_learning_doc_yaml)
}
