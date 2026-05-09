use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{OrbitError, TaskStatus, TaskType};

use super::TaskFileStore;
use crate::file::layout::{ensure_dirs, read_child_dirs as list_child_dirs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskStateDir {
    Proposed,
    Friction,
    Backlog,
    Someday,
    InProgress,
    Review,
    Done,
    Blocked,
    Archived,
    Rejected,
}

impl TaskStateDir {
    pub(super) fn as_dir(self) -> &'static str {
        match self {
            TaskStateDir::Proposed => "proposed",
            TaskStateDir::Friction => "friction",
            TaskStateDir::Backlog => "backlog",
            TaskStateDir::Someday => "someday",
            TaskStateDir::InProgress => "in_progress",
            TaskStateDir::Review => "review",
            TaskStateDir::Done => "done",
            TaskStateDir::Blocked => "blocked",
            TaskStateDir::Archived => "archived",
            TaskStateDir::Rejected => "rejected",
        }
    }

    pub(super) fn to_status(self) -> TaskStatus {
        match self {
            TaskStateDir::Proposed => TaskStatus::Proposed,
            TaskStateDir::Friction => TaskStatus::Friction,
            TaskStateDir::Backlog => TaskStatus::Backlog,
            TaskStateDir::Someday => TaskStatus::Someday,
            TaskStateDir::InProgress => TaskStatus::InProgress,
            TaskStateDir::Review => TaskStatus::Review,
            TaskStateDir::Done => TaskStatus::Done,
            TaskStateDir::Blocked => TaskStatus::Blocked,
            TaskStateDir::Archived => TaskStatus::Archived,
            TaskStateDir::Rejected => TaskStatus::Rejected,
        }
    }

    pub(super) fn from_status(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Proposed => TaskStateDir::Proposed,
            TaskStatus::Friction => TaskStateDir::Friction,
            TaskStatus::Backlog => TaskStateDir::Backlog,
            TaskStatus::Someday => TaskStateDir::Someday,
            TaskStatus::InProgress => TaskStateDir::InProgress,
            TaskStatus::Review => TaskStateDir::Review,
            TaskStatus::Done => TaskStateDir::Done,
            TaskStatus::Blocked => TaskStateDir::Blocked,
            TaskStatus::Archived => TaskStateDir::Archived,
            TaskStatus::Rejected => TaskStateDir::Rejected,
        }
    }

    pub(super) fn all() -> [TaskStateDir; 10] {
        [
            TaskStateDir::Proposed,
            TaskStateDir::Friction,
            TaskStateDir::Backlog,
            TaskStateDir::Someday,
            TaskStateDir::InProgress,
            TaskStateDir::Review,
            TaskStateDir::Done,
            TaskStateDir::Blocked,
            TaskStateDir::Archived,
            TaskStateDir::Rejected,
        ]
    }

    pub(super) fn is_partitioned(self) -> bool {
        matches!(
            self,
            TaskStateDir::Done | TaskStateDir::Archived | TaskStateDir::Rejected
        )
    }
}

impl TaskFileStore {
    pub(super) fn ensure_layout(&self) -> Result<(), OrbitError> {
        let dirs = TaskStateDir::all()
            .map(|state| self.root.join(state.as_dir()))
            .to_vec();
        let dir_refs = dirs.iter().map(|dir| dir.as_path()).collect::<Vec<_>>();
        ensure_dirs(&dir_refs)
    }

    pub(super) fn next_task_id(&self, now: DateTime<Utc>) -> Result<String, OrbitError> {
        // Caller must hold acquire_task_allocation_lock while scanning and then creating the task
        // directory, so the scan-then-allocate window remains serialized.
        let task_date = now.format("%Y%m%d").to_string();
        let today_prefix = format!("T{task_date}-");
        let current_partition = now.format("%Y-%m").to_string();
        let mut max_suffix = 0_u32;

        for state in TaskStateDir::all() {
            let state_dir = self.state_dir_path(state);
            if !state_dir.exists() {
                continue;
            }

            if state.is_partitioned() {
                update_max_suffix_from_child_dirs(&state_dir, &today_prefix, &mut max_suffix)?;
                update_max_suffix_from_child_dirs(
                    &state_dir.join(&current_partition),
                    &today_prefix,
                    &mut max_suffix,
                )?;
            } else {
                update_max_suffix_from_child_dirs(&state_dir, &today_prefix, &mut max_suffix)?;
            }
        }

        let next_suffix = max_suffix
            .checked_add(1)
            .ok_or_else(|| OrbitError::Execution("task id counter overflow".to_string()))?;
        Ok(format!("T{task_date}-{next_suffix}"))
    }

    pub(super) fn locate_task(
        &self,
        id: &str,
    ) -> Result<Option<(TaskStateDir, PathBuf)>, OrbitError> {
        validate_task_id(id)?;

        for state in TaskStateDir::all() {
            if state.is_partitioned() {
                if let Some(partition) = partition_key(id) {
                    let partitioned_dir = self.state_dir_path(state).join(partition).join(id);
                    if partitioned_dir.is_dir() {
                        return Ok(Some((state, partitioned_dir)));
                    }
                }

                let legacy_dir = self.state_dir_path(state).join(id);
                if legacy_dir.is_dir() {
                    let migrated_dir = self.migrate_legacy_task_dir(state, legacy_dir)?;
                    return Ok(Some((state, migrated_dir)));
                }

                for partition_dir in self.partition_dirs(state)? {
                    let partitioned_dir = partition_dir.join(id);
                    if partitioned_dir.is_dir() {
                        return Ok(Some((state, partitioned_dir)));
                    }
                }
                continue;
            }

            let task_dir = self.task_dir(state, id);
            if task_dir.is_dir() {
                if state == TaskStateDir::Proposed
                    && let Some(migrated) =
                        self.migrate_legacy_proposed_friction_task_dir(task_dir.clone())?
                {
                    return Ok(Some((TaskStateDir::Friction, migrated)));
                }
                return Ok(Some((state, task_dir)));
            }
        }
        Ok(None)
    }

    pub(super) fn state_dir_path(&self, state: TaskStateDir) -> PathBuf {
        self.root.join(state.as_dir())
    }

    pub(super) fn task_dir(&self, state: TaskStateDir, id: &str) -> PathBuf {
        if state.is_partitioned()
            && let Some(partition) = partition_key(id)
        {
            return self.state_dir_path(state).join(partition).join(id);
        }
        self.state_dir_path(state).join(id)
    }

    pub(super) fn task_doc_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(super::constants::TASK_DOC_FILE_NAME)
    }

    pub(super) fn plan_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(super::constants::PLAN_FILE_NAME)
    }

    pub(super) fn execution_summary_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(super::constants::EXECUTION_SUMMARY_FILE_NAME)
    }

    pub(super) fn artifacts_dir(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(super::constants::ARTIFACTS_DIR_NAME)
    }

    pub(super) fn task_dirs_for_state(
        &self,
        state: TaskStateDir,
    ) -> Result<Vec<PathBuf>, OrbitError> {
        if state == TaskStateDir::Friction {
            self.migrate_legacy_proposed_friction_tasks()?;
        }

        let state_dir = self.state_dir_path(state);
        if !state_dir.exists() {
            return Ok(Vec::new());
        }

        if !state.is_partitioned() {
            return list_child_dirs(&state_dir);
        }

        let mut task_dirs = Vec::new();
        for entry in list_child_dirs(&state_dir)? {
            let Some(name) = entry.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            if is_partition_dir_name(name) {
                task_dirs.extend(list_child_dirs(&entry)?);
                continue;
            }

            if self.task_doc_path(&entry).is_file() {
                task_dirs.push(self.migrate_legacy_task_dir(state, entry)?);
            }
        }

        Ok(task_dirs)
    }

    pub(super) fn migrate_legacy_proposed_friction_tasks(&self) -> Result<(), OrbitError> {
        let proposed_dir = self.state_dir_path(TaskStateDir::Proposed);
        if !proposed_dir.exists() {
            return Ok(());
        }

        for task_dir in list_child_dirs(&proposed_dir)? {
            let _ = self.migrate_legacy_proposed_friction_task_dir(task_dir)?;
        }
        Ok(())
    }

    pub(super) fn migrate_legacy_proposed_friction_task_dir(
        &self,
        task_dir: PathBuf,
    ) -> Result<Option<PathBuf>, OrbitError> {
        if !self.task_doc_path(&task_dir).is_file() {
            return Ok(None);
        }
        let mut bundle = self.read_bundle_at(&task_dir)?;
        if bundle.doc.task_type != TaskType::Friction {
            return Ok(None);
        }

        let task_id = bundle.doc.id.clone();
        let target_dir = self.task_dir(TaskStateDir::Friction, &task_id);
        if target_dir == task_dir {
            return Ok(Some(task_dir));
        }
        if target_dir.exists() {
            return Err(OrbitError::Store(format!(
                "cannot migrate friction task directory {} because {} already exists",
                task_dir.display(),
                target_dir.display()
            )));
        }

        rewrite_legacy_proposed_friction_history(&mut bundle.doc.history);
        self.write_bundle_at(&task_dir, &bundle)?;
        self.move_task_dir(&task_dir, &target_dir)?;
        Ok(Some(target_dir))
    }

    pub(super) fn partition_dirs(&self, state: TaskStateDir) -> Result<Vec<PathBuf>, OrbitError> {
        let state_dir = self.state_dir_path(state);
        if !state_dir.exists() {
            return Ok(Vec::new());
        }

        Ok(list_child_dirs(&state_dir)?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_partition_dir_name)
            })
            .collect())
    }

    pub(super) fn migrate_legacy_task_dir(
        &self,
        state: TaskStateDir,
        legacy_dir: PathBuf,
    ) -> Result<PathBuf, OrbitError> {
        let Some(task_id) = legacy_dir.file_name().and_then(|value| value.to_str()) else {
            return Err(OrbitError::Store(format!(
                "invalid task directory path {}",
                legacy_dir.display()
            )));
        };
        validate_task_id(task_id)?;
        let target_dir = self.task_dir(state, task_id);
        if target_dir == legacy_dir {
            return Ok(legacy_dir);
        }
        if target_dir.exists() {
            return Err(OrbitError::Store(format!(
                "cannot migrate task directory {} because {} already exists",
                legacy_dir.display(),
                target_dir.display()
            )));
        }
        self.move_task_dir(&legacy_dir, &target_dir)?;
        Ok(target_dir)
    }

    pub(super) fn move_task_dir(&self, from: &Path, to: &Path) -> Result<(), OrbitError> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        fs::rename(from, to).map_err(|e| OrbitError::Io(e.to_string()))
    }
}

pub(super) fn validate_task_id(id: &str) -> Result<(), OrbitError> {
    if is_valid_task_id(id) {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(
        "task id must match TYYYYMMDD-<digits> with optional legacy -<digits> suffix".to_string(),
    ))
}

fn rewrite_legacy_proposed_friction_history(history: &mut [orbit_common::types::TaskHistoryEntry]) {
    for entry in history {
        if entry.event == "created"
            && entry.from_status.is_none()
            && entry.to_status == Some(TaskStatus::Proposed)
        {
            entry.to_status = Some(TaskStatus::Friction);
            return;
        }
    }
}

fn is_valid_task_id(id: &str) -> bool {
    let Some(raw) = id.strip_prefix('T') else {
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
    if !is_valid_year_month(year, month) {
        return false;
    }

    let Some(tail) = raw.get(8..).and_then(|value| value.strip_prefix('-')) else {
        return false;
    };
    !tail.is_empty()
        && tail.split('-').all(|component| {
            !component.is_empty() && component.as_bytes().iter().all(u8::is_ascii_digit)
        })
}

fn partition_key(id: &str) -> Option<String> {
    let raw = id.strip_prefix('T')?;
    let year = raw.get(0..4)?;
    let month = raw.get(4..6)?;
    is_valid_year_month(year, month).then(|| format!("{year}-{month}"))
}

fn update_max_suffix_from_child_dirs(
    dir: &Path,
    today_prefix: &str,
    max_suffix: &mut u32,
) -> Result<(), OrbitError> {
    if !dir.exists() {
        return Ok(());
    }

    for child in list_child_dirs(dir)? {
        let Some(name) = child.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(tail) = name.strip_prefix(today_prefix) else {
            continue;
        };
        *max_suffix = (*max_suffix).max(extract_max_suffix_component(tail));
    }
    Ok(())
}

/// Parse the suffix portion of a task ID (everything after `T{YYYYMMDD}-`).
///
/// Compound legacy IDs contribute their largest integer component, not their
/// trailing one, so `2313-2` advances the next counter to `2314`.
fn extract_max_suffix_component(tail: &str) -> u32 {
    tail.split('-')
        .filter_map(|component| component.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

fn is_partition_dir_name(name: &str) -> bool {
    let Some((year, month)) = name.split_once('-') else {
        return false;
    };
    year.len() == 4 && month.len() == 2 && is_valid_year_month(year, month)
}

fn is_valid_year_month(year: &str, month: &str) -> bool {
    year.as_bytes().iter().all(u8::is_ascii_digit)
        && matches!(
            month,
            "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
        )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        fs,
        sync::{Arc, Barrier},
        thread,
    };

    use chrono::TimeZone;
    use regex::Regex;
    use tempfile::{TempDir, tempdir};

    use super::*;

    fn fixture() -> (TempDir, TaskFileStore, DateTime<Utc>) {
        let tempdir = tempdir().expect("task store tempdir");
        let store = TaskFileStore::new(tempdir.path().to_path_buf());
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        (tempdir, store, now)
    }

    fn create_task_dir(store: &TaskFileStore, state: TaskStateDir, id: &str) -> PathBuf {
        let task_dir = store.task_dir(state, id);
        fs::create_dir_all(&task_dir).expect("create task dir");
        task_dir
    }

    #[test]
    fn next_task_id_fresh_workspace_returns_one() {
        let (_tempdir, store, now) = fixture();

        let id = store.next_task_id(now).expect("next task id");

        assert_eq!(id, "T20260426-1");
    }

    #[test]
    fn next_task_id_format_matches_regex() {
        let (_tempdir, store, now) = fixture();

        let id = store.next_task_id(now).expect("next task id");
        let pattern = Regex::new(r"^T\d{8}-[1-9]\d*$").expect("valid regex");

        assert!(pattern.is_match(&id), "{id} should match task id format");
    }

    #[test]
    fn next_task_id_sequential_increments() {
        let (_tempdir, store, now) = fixture();
        let mut ids = Vec::new();

        for _ in 0..5 {
            let id = store.next_task_id(now).expect("next task id");
            create_task_dir(&store, TaskStateDir::Backlog, &id);
            ids.push(id);
        }

        assert_eq!(
            ids,
            vec![
                "T20260426-1",
                "T20260426-2",
                "T20260426-3",
                "T20260426-4",
                "T20260426-5",
            ]
        );
    }

    #[test]
    fn next_task_id_with_legacy_hhmm_continues_from_max() {
        let (_tempdir, store, now) = fixture();
        create_task_dir(&store, TaskStateDir::Backlog, "T20260426-2313");

        let id = store.next_task_id(now).expect("next task id");

        assert_eq!(id, "T20260426-2314");
    }

    #[test]
    fn next_task_id_with_compound_legacy_id_continues_from_largest_component() {
        let (_tempdir, store, now) = fixture();
        create_task_dir(&store, TaskStateDir::Backlog, "T20260426-2313-2");

        let id = store.next_task_id(now).expect("next task id");

        assert_eq!(id, "T20260426-2314");
    }

    #[test]
    fn next_task_id_ignores_other_days() {
        let (_tempdir, store, now) = fixture();
        create_task_dir(&store, TaskStateDir::Backlog, "T20260425-9999");
        create_task_dir(&store, TaskStateDir::Backlog, "T20260427-1");

        let id = store.next_task_id(now).expect("next task id");

        assert_eq!(id, "T20260426-1");
    }

    #[test]
    fn next_task_id_scans_all_states() {
        let (_tempdir, store, now) = fixture();
        create_task_dir(&store, TaskStateDir::Done, "T20260426-5");
        create_task_dir(&store, TaskStateDir::Proposed, "T20260426-3");

        let id = store.next_task_id(now).expect("next task id");

        assert_eq!(id, "T20260426-6");
    }

    #[test]
    fn locate_task_resolves_all_three_id_forms() {
        let (_tempdir, store, _now) = fixture();
        let task_ids = ["T20260426-1", "T20260426-2313", "T20260426-2313-2"];

        for id in task_ids {
            let expected_dir = create_task_dir(&store, TaskStateDir::Done, id);
            let located = store.locate_task(id).expect("locate task");

            assert_eq!(located, Some((TaskStateDir::Done, expected_dir)));
        }
    }

    #[test]
    fn locate_task_migrates_legacy_partitioned_id_forms() {
        let (_tempdir, store, _now) = fixture();
        let task_ids = ["T20260426-1", "T20260426-0455", "T20260426-2313-2"];

        for id in task_ids {
            let legacy_dir = store.state_dir_path(TaskStateDir::Done).join(id);
            fs::create_dir_all(&legacy_dir).expect("create legacy task dir");
            let expected_dir = store.task_dir(TaskStateDir::Done, id);

            let located = store.locate_task(id).expect("locate task");

            assert_eq!(located, Some((TaskStateDir::Done, expected_dir.clone())));
            assert!(expected_dir.is_dir(), "{id} should be migrated");
            assert!(!legacy_dir.exists(), "{id} legacy dir should be moved");
        }
    }

    #[test]
    fn locate_task_rejects_invalid_path_like_ids_before_path_lookup() {
        let (_tempdir, store, _now) = fixture();
        let invalid_ids = [
            "",
            "   ",
            "../T20260426-1",
            "../../outside",
            "/tmp/T20260426-1",
            r"C:\tmp\T20260426-1",
            r"T20260426-1\outside",
            "T20260426-1/outside",
            "T20260426-1..2",
        ];

        for id in invalid_ids {
            let err = store.locate_task(id).expect_err("invalid id rejected");
            assert!(
                matches!(err, OrbitError::InvalidInput(_)),
                "{id:?} should fail as invalid input, got {err:?}"
            );
        }
    }

    #[test]
    fn partition_key_uniform_across_id_forms() {
        let partition = Some("2026-04".to_string());

        assert_eq!(partition_key("T20260426-1"), partition);
        assert_eq!(partition_key("T20260426-2313"), partition);
        assert_eq!(partition_key("T20260426-2313-2"), partition);
    }

    #[test]
    fn next_task_id_concurrent_allocation_lock_yields_distinct_ids() {
        let (_tempdir, store, now) = fixture();
        let store = Arc::new(store);
        let barrier = Arc::new(Barrier::new(16));

        let handles = (0..16)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let _allocation_lock = store
                        .acquire_task_allocation_lock()
                        .expect("task allocation lock");
                    let id = store.next_task_id(now).expect("next task id");
                    create_task_dir(&store, TaskStateDir::Backlog, &id);
                    id
                })
            })
            .collect::<Vec<_>>();

        let ids = handles
            .into_iter()
            .map(|handle| handle.join().expect("allocation thread"))
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), 16);
    }
}
