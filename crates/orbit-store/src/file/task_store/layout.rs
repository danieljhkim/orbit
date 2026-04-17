use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{OrbitError, TaskStatus};

use super::TaskFileStore;
use crate::file::layout::{ensure_dirs, read_child_dirs as list_child_dirs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskStateDir {
    Proposed,
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

    pub(super) fn all() -> [TaskStateDir; 9] {
        [
            TaskStateDir::Proposed,
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
        let base = format!("T{}", now.format("%Y%m%d-%H%M"));
        if self.locate_task(&base)?.is_none() {
            return Ok(base);
        }
        for suffix in 2..1024_u32 {
            let candidate = format!("{base}-{suffix}");
            if self.locate_task(&candidate)?.is_none() {
                return Ok(candidate);
            }
        }
        Err(OrbitError::Execution(
            "unable to allocate unique task id".to_string(),
        ))
    }

    pub(super) fn locate_task(
        &self,
        id: &str,
    ) -> Result<Option<(TaskStateDir, PathBuf)>, OrbitError> {
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
                return Ok(Some((state, task_dir)));
            }
        }
        Ok(None)
    }

    pub(super) fn state_dir_path(&self, state: TaskStateDir) -> PathBuf {
        self.root.join(state.as_dir())
    }

    pub(super) fn task_dir(&self, state: TaskStateDir, id: &str) -> PathBuf {
        if state.is_partitioned() {
            if let Some(partition) = partition_key(id) {
                return self.state_dir_path(state).join(partition).join(id);
            }
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

fn partition_key(id: &str) -> Option<String> {
    let raw = id.strip_prefix('T')?;
    let year = raw.get(0..4)?;
    let month = raw.get(4..6)?;
    is_valid_year_month(year, month).then(|| format!("{year}-{month}"))
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
