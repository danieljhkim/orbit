use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;

use fs2::FileExt;
use orbit_common::types::OrbitError;

use super::{TaskFileStore, layout::validate_task_id};

const LOCKS_DIR_NAME: &str = ".locks";
const TASK_LOCKS_DIR_NAME: &str = "tasks";
const TASK_CREATE_LOCK_FILE_NAME: &str = "task-create.lock";

impl TaskFileStore {
    pub(super) fn acquire_task_allocation_lock(&self) -> Result<File, OrbitError> {
        self.acquire_lock(
            self.store_lock_path(TASK_CREATE_LOCK_FILE_NAME),
            "task allocation",
        )
    }

    pub(super) fn acquire_task_lock(&self, task_id: &str) -> Result<File, OrbitError> {
        validate_task_id(task_id)?;
        self.acquire_lock(self.task_lock_path(task_id), &format!("task '{task_id}'"))
    }

    fn acquire_lock(&self, path: PathBuf, label: &str) -> Result<File, OrbitError> {
        let parent = path.parent().ok_or_else(|| {
            OrbitError::Store(format!(
                "cannot determine lock parent for '{}'",
                path.display()
            ))
        })?;
        fs::create_dir_all(parent).map_err(|err| OrbitError::Io(err.to_string()))?;

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|err| OrbitError::Io(err.to_string()))?;
        file.lock_exclusive().map_err(|err| {
            OrbitError::Store(format!(
                "failed to acquire {label} lock '{}': {err}",
                path.display()
            ))
        })?;
        Ok(file)
    }

    fn lock_root(&self) -> PathBuf {
        self.root.join(LOCKS_DIR_NAME)
    }

    fn store_lock_path(&self, file_name: &str) -> PathBuf {
        self.lock_root().join(file_name)
    }

    fn task_lock_path(&self, task_id: &str) -> PathBuf {
        self.lock_root()
            .join(TASK_LOCKS_DIR_NAME)
            .join(format!("{task_id}.lock"))
    }
}
