use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{LearningStatus, OrbitError};

use super::constants::{LEARNING_DOC_FILE_EXT, SUPERSEDED_DIR_NAME};

/// Filesystem state directories for learnings.
///
/// Mirrors the `task_store::TaskStateDir` shape but with the smaller phase-1
/// lifecycle (active records live at the root, superseded ones move under
/// `superseded/`). Both directories are checked into git per ADR-003.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LearningStateDir {
    Active,
    Superseded,
}

impl LearningStateDir {
    pub(super) fn all() -> &'static [LearningStateDir] {
        &[LearningStateDir::Active, LearningStateDir::Superseded]
    }

    pub(super) fn to_status(self) -> LearningStatus {
        match self {
            LearningStateDir::Active => LearningStatus::Active,
            LearningStateDir::Superseded => LearningStatus::Superseded,
        }
    }

    #[allow(dead_code)]
    pub(super) fn from_status(status: LearningStatus) -> Self {
        match status {
            LearningStatus::Active => LearningStateDir::Active,
            LearningStatus::Superseded => LearningStateDir::Superseded,
        }
    }
}

/// Directory containing learning records for the given state.
///
/// - `Active` lives directly under the store root (e.g. `.orbit/learnings/`)
///   so PR diffs surface the most-frequently-touched records first.
/// - `Superseded` lives under `<root>/superseded/` per ADR-003.
pub(super) fn state_dir_path(root: &Path, state: LearningStateDir) -> PathBuf {
    match state {
        LearningStateDir::Active => root.to_path_buf(),
        LearningStateDir::Superseded => root.join(SUPERSEDED_DIR_NAME),
    }
}

pub(super) fn learning_doc_path(root: &Path, state: LearningStateDir, id: &str) -> PathBuf {
    state_dir_path(root, state).join(format!("{id}.{LEARNING_DOC_FILE_EXT}"))
}

/// Locate the `(state, file_path)` of a learning by id, or `None` if missing.
pub(super) fn locate_learning(
    root: &Path,
    id: &str,
) -> Result<Option<(LearningStateDir, PathBuf)>, OrbitError> {
    validate_learning_id(id)?;
    for state in LearningStateDir::all() {
        let path = learning_doc_path(root, *state, id);
        if path.is_file() {
            return Ok(Some((*state, path)));
        }
    }
    Ok(None)
}

/// Move the YAML file from one state directory to another (e.g. `active` →
/// `superseded`). Creates the destination parent directory if missing.
pub(super) fn move_learning_dir(from: &Path, to: &Path) -> Result<(), OrbitError> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    fs::rename(from, to).map_err(|e| OrbitError::Io(e.to_string()))
}

/// Allocate the next sequential learning id of the form `L<YYYYMMDD>-<NNNN>`.
///
/// `<NNNN>` is monotonically increasing across active and superseded records
/// for the given day; allocation rolls over each calendar day.
///
/// **Caller contract**: must hold an allocation lock (see
/// [`super::lock::acquire_learning_allocation_lock`]) for the duration of
/// the scan and the subsequent file creation, so the scan-then-allocate
/// window remains serialized across concurrent writers.
pub(super) fn next_learning_id(root: &Path, now: DateTime<Utc>) -> Result<String, OrbitError> {
    let date = now.format("%Y%m%d").to_string();
    let prefix = format!("L{date}-");
    let mut max_suffix: u32 = 0;

    for state in LearningStateDir::all() {
        let dir = state_dir_path(root, *state);
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|e| OrbitError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(&format!(".{LEARNING_DOC_FILE_EXT}")) else {
                continue;
            };
            let Some(tail) = stem.strip_prefix(&prefix) else {
                continue;
            };
            if let Ok(n) = tail.parse::<u32>() {
                max_suffix = max_suffix.max(n);
            }
        }
    }

    let next = max_suffix
        .checked_add(1)
        .ok_or_else(|| OrbitError::Execution("learning id counter overflow".to_string()))?;
    Ok(format!("L{date}-{next}"))
}

/// Validate that `id` is shaped as `L<YYYYMMDD>-<digits>` and free of path
/// traversal characters.
pub(super) fn validate_learning_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_learning_id(id) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "learning id must match L<YYYYMMDD>-<digits>: {id}"
    )))
}

fn is_valid_learning_id(id: &str) -> bool {
    let Some(raw) = id.strip_prefix('L') else {
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
    let Some(year) = date.get(0..4) else {
        return false;
    };
    let Some(month) = date.get(4..6) else {
        return false;
    };
    if !year.as_bytes().iter().all(u8::is_ascii_digit) {
        return false;
    }
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
mod tests {
    use std::fs;

    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn next_learning_id_on_empty_root_is_one() {
        let dir = tempdir().expect("tempdir");
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        let id = next_learning_id(dir.path(), now).expect("next id");
        assert_eq!(id, "L20260511-1");
    }

    #[test]
    fn next_learning_id_scans_active_and_superseded_dirs() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("L20260511-1.yaml"), "").expect("seed active");
        fs::create_dir_all(dir.path().join("superseded")).expect("mk superseded");
        fs::write(dir.path().join("superseded").join("L20260511-3.yaml"), "")
            .expect("seed superseded");

        let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        let id = next_learning_id(dir.path(), now).expect("next id");
        assert_eq!(id, "L20260511-4");
    }

    #[test]
    fn next_learning_id_ignores_other_days() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("L20260510-99.yaml"), "").expect("seed yesterday");
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
        let id = next_learning_id(dir.path(), now).expect("next id");
        assert_eq!(id, "L20260511-1");
    }

    #[test]
    fn locate_learning_finds_record_in_either_state() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("superseded")).expect("mk superseded");
        fs::write(dir.path().join("L20260511-1.yaml"), "").expect("active");
        fs::write(dir.path().join("superseded").join("L20260511-2.yaml"), "").expect("superseded");

        let (state, path) = locate_learning(dir.path(), "L20260511-1")
            .expect("locate")
            .expect("found");
        assert_eq!(state, LearningStateDir::Active);
        assert_eq!(path, dir.path().join("L20260511-1.yaml"));

        let (state, path) = locate_learning(dir.path(), "L20260511-2")
            .expect("locate")
            .expect("found");
        assert_eq!(state, LearningStateDir::Superseded);
        assert_eq!(path, dir.path().join("superseded").join("L20260511-2.yaml"));
    }

    #[test]
    fn validate_learning_id_accepts_well_formed_ids() {
        assert!(validate_learning_id("L20260511-1").is_ok());
        assert!(validate_learning_id("L20260511-9999").is_ok());
    }

    #[test]
    fn validate_learning_id_rejects_path_like_ids() {
        for bad in [
            "",
            "  ",
            "T20260511-1",
            "L20261311-1",
            "L20260511-",
            "L20260511-1/escape",
            "../L20260511-1",
        ] {
            assert!(
                validate_learning_id(bad).is_err(),
                "expected reject for {bad:?}"
            );
        }
    }
}
