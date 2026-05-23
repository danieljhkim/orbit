//! Shared legacy session-state file helpers for project-learning import.

use std::fs;
use std::path::Path;

use crate::types::{LearningInjectionState, OrbitError};

/// Retained for legacy import only; production state now lives in SQLite.
pub const LEARNING_SESSION_STATE_FILE_NAME: &str = "learnings.json";

/// Retained for legacy import only; production reads now go through SQLite.
pub fn read_learning_session_state(
    path: &Path,
) -> Result<Option<LearningInjectionState>, OrbitError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        OrbitError::Store(format!(
            "read learning session state '{}': {error}",
            path.display()
        ))
    })?;
    if raw.trim().is_empty() {
        return Ok(Some(LearningInjectionState::default()));
    }
    serde_json::from_str(&raw).map(Some).map_err(|error| {
        OrbitError::Store(format!(
            "parse learning session state '{}': {error}",
            path.display()
        ))
    })
}
