//! Shared session-state file helpers for project-learning injection.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::types::{LearningInjectionState, OrbitError};

/// Retained for legacy import only; production writes now go through SQLite.
pub const LEARNING_SESSION_STATE_RELATIVE_DIR: &str = ".orbit/state/sessions";
/// Retained for legacy import only; production writes now go through SQLite.
pub const LEARNING_SESSION_STATE_FILE_NAME: &str = "learnings.json";

/// Retained for legacy import only; production writes now go through SQLite.
pub fn learning_session_state_path(workspace_root: &Path, session_id: &str) -> PathBuf {
    workspace_root
        .join(LEARNING_SESSION_STATE_RELATIVE_DIR)
        .join(session_id)
        .join(LEARNING_SESSION_STATE_FILE_NAME)
}

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

/// Retained for legacy import only; production writes now go through SQLite.
pub fn write_learning_session_state(
    path: &Path,
    state: &LearningInjectionState,
) -> Result<(), OrbitError> {
    update_learning_session_state(path, |existing| {
        *existing = state.clone();
    })
    .map(|_| ())
}

/// Retained for legacy import only; production writes now go through SQLite.
pub fn update_learning_session_state<R>(
    path: &Path,
    update: impl FnOnce(&mut LearningInjectionState) -> R,
) -> Result<(LearningInjectionState, R), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Store(format!(
                "create learning session state dir '{}': {error}",
                parent.display()
            ))
        })?;
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            OrbitError::Store(format!(
                "open learning session state '{}': {error}",
                path.display()
            ))
        })?;
    file.lock_exclusive().map_err(|error| {
        OrbitError::Store(format!(
            "lock learning session state '{}': {error}",
            path.display()
        ))
    })?;

    let result = update_locked_state(&mut file, path, update);
    let unlock_result = file.unlock().map_err(|error| {
        OrbitError::Store(format!(
            "unlock learning session state '{}': {error}",
            path.display()
        ))
    });

    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn update_locked_state<R>(
    file: &mut fs::File,
    path: &Path,
    update: impl FnOnce(&mut LearningInjectionState) -> R,
) -> Result<(LearningInjectionState, R), OrbitError> {
    let mut raw = String::new();
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        OrbitError::Store(format!(
            "seek learning session state '{}': {error}",
            path.display()
        ))
    })?;
    file.read_to_string(&mut raw).map_err(|error| {
        OrbitError::Store(format!(
            "read learning session state '{}': {error}",
            path.display()
        ))
    })?;
    let mut state = if raw.trim().is_empty() {
        LearningInjectionState::default()
    } else {
        serde_json::from_str(&raw).map_err(|error| {
            OrbitError::Store(format!(
                "parse learning session state '{}': {error}",
                path.display()
            ))
        })?
    };

    let callback_result = update(&mut state);
    let encoded = serde_json::to_vec_pretty(&state).map_err(|error| {
        OrbitError::Store(format!(
            "serialize learning session state '{}': {error}",
            path.display()
        ))
    })?;
    file.set_len(0).map_err(|error| {
        OrbitError::Store(format!(
            "truncate learning session state '{}': {error}",
            path.display()
        ))
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        OrbitError::Store(format!(
            "seek learning session state '{}': {error}",
            path.display()
        ))
    })?;
    file.write_all(&encoded).map_err(|error| {
        OrbitError::Store(format!(
            "write learning session state '{}': {error}",
            path.display()
        ))
    })?;
    file.write_all(b"\n").map_err(|error| {
        OrbitError::Store(format!(
            "write learning session state '{}': {error}",
            path.display()
        ))
    })?;

    Ok((state, callback_result))
}
