use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use orbit_types::OrbitError;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileLock {
    pub file_path: String,
    pub task_id: String,
    pub repo_root: String,
    pub acquired_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileLockConflict {
    pub file_path: String,
    pub held_by_task_id: String,
}

pub trait FileLockChecker: Send + Sync {
    fn check_write_allowed(
        &self,
        task_id: Option<&str>,
        repo_root: &str,
        file_path: &str,
    ) -> Result<(), OrbitError>;

    fn auto_acquire(
        &self,
        task_id: &str,
        repo_root: &str,
        file_path: &str,
    ) -> Result<(), OrbitError>;
}

#[derive(Clone)]
pub struct FileLockStore {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for FileLockStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLockStore").finish_non_exhaustive()
    }
}

impl FileLockStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn acquire_locks(
        &self,
        task_id: &str,
        repo_root: &str,
        paths: &[&str],
    ) -> Result<(), OrbitError> {
        if paths.is_empty() {
            return Ok(());
        }

        let mut conn = self.lock_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| OrbitError::Store(error.to_string()))?;

        let conflicts = Self::check_conflicts_in_conn(&tx, repo_root, paths, Some(task_id))?;
        if !conflicts.is_empty() {
            return Err(conflict_error(task_id, &conflicts));
        }

        for path in dedupe_paths(paths) {
            tx.execute(
                "INSERT OR IGNORE INTO file_locks (file_path, task_id, repo_root, acquired_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![path, task_id, repo_root, Utc::now().to_rfc3339()],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        }

        tx.commit()
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    pub fn release_locks_for_task(&self, task_id: &str) -> Result<usize, OrbitError> {
        let conn = self.lock_connection()?;
        let deleted = conn
            .execute(
                "DELETE FROM file_locks WHERE task_id = ?1",
                params![task_id],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(deleted)
    }

    pub fn lock_holder(
        &self,
        repo_root: &str,
        file_path: &str,
    ) -> Result<Option<FileLock>, OrbitError> {
        let conn = self.lock_connection()?;
        conn.query_row(
            "SELECT file_path, task_id, repo_root, acquired_at
             FROM file_locks
             WHERE repo_root = ?1 AND file_path = ?2",
            params![repo_root, file_path],
            |row| {
                let acquired_at: String = row.get(3)?;
                let parsed = DateTime::parse_from_rfc3339(&acquired_at).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(FileLock {
                    file_path: row.get(0)?,
                    task_id: row.get(1)?,
                    repo_root: row.get(2)?,
                    acquired_at: parsed.with_timezone(&Utc),
                })
            },
        )
        .optional()
        .map_err(|error| OrbitError::Store(error.to_string()))
    }

    pub fn check_conflicts(
        &self,
        repo_root: &str,
        paths: &[&str],
        exclude_task_id: Option<&str>,
    ) -> Result<Vec<FileLockConflict>, OrbitError> {
        let conn = self.lock_connection()?;
        Self::check_conflicts_in_conn(&conn, repo_root, paths, exclude_task_id)
    }

    pub fn acquire_single_lock(
        &self,
        task_id: &str,
        repo_root: &str,
        file_path: &str,
    ) -> Result<(), OrbitError> {
        let conflicts = self.check_conflicts(repo_root, &[file_path], Some(task_id))?;
        if !conflicts.is_empty() {
            return Err(conflict_error(task_id, &conflicts));
        }

        let conn = self.lock_connection()?;
        conn.execute(
            "INSERT OR IGNORE INTO file_locks (file_path, task_id, repo_root, acquired_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![file_path, task_id, repo_root, Utc::now().to_rfc3339()],
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    pub fn release_stale_locks(&self, active_task_ids: &[&str]) -> Result<usize, OrbitError> {
        let conn = self.lock_connection()?;
        let deleted = if active_task_ids.is_empty() {
            conn.execute("DELETE FROM file_locks", [])
        } else {
            let placeholders = (0..active_task_ids.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("DELETE FROM file_locks WHERE task_id NOT IN ({placeholders})");
            conn.execute(
                &sql,
                rusqlite::params_from_iter(active_task_ids.iter().copied()),
            )
        }
        .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(deleted)
    }

    fn check_conflicts_in_conn(
        conn: &Connection,
        repo_root: &str,
        paths: &[&str],
        exclude_task_id: Option<&str>,
    ) -> Result<Vec<FileLockConflict>, OrbitError> {
        let mut conflicts = Vec::new();
        for path in dedupe_paths(paths) {
            let holder: Option<String> = conn
                .query_row(
                    "SELECT task_id
                     FROM file_locks
                     WHERE repo_root = ?1 AND file_path = ?2",
                    params![repo_root, path],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            if let Some(held_by_task_id) = holder
                && exclude_task_id.is_none_or(|task_id| task_id != held_by_task_id)
            {
                conflicts.push(FileLockConflict {
                    file_path: path.to_string(),
                    held_by_task_id,
                });
            }
        }
        Ok(conflicts)
    }

    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, OrbitError> {
        self.conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))
    }
}

impl FileLockChecker for FileLockStore {
    fn check_write_allowed(
        &self,
        task_id: Option<&str>,
        repo_root: &str,
        file_path: &str,
    ) -> Result<(), OrbitError> {
        let holder = self.lock_holder(repo_root, file_path)?;
        match (task_id, holder) {
            (_, None) => Ok(()),
            (Some(task_id), Some(lock)) if lock.task_id == task_id => Ok(()),
            (Some(task_id), Some(lock)) => Err(OrbitError::PolicyDenied(format!(
                "task '{task_id}' cannot modify '{file_path}' because it is locked by task '{}'",
                lock.task_id
            ))),
            (None, Some(lock)) => Err(OrbitError::PolicyDenied(format!(
                "write to '{file_path}' requires a task context because it is locked by task '{}'",
                lock.task_id
            ))),
        }
    }

    fn auto_acquire(
        &self,
        task_id: &str,
        repo_root: &str,
        file_path: &str,
    ) -> Result<(), OrbitError> {
        self.acquire_single_lock(task_id, repo_root, file_path)
    }
}

fn dedupe_paths<'a>(paths: &'a [&'a str]) -> Vec<&'a str> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.iter().any(|existing| existing == path) {
            deduped.push(*path);
        }
    }
    deduped
}

fn conflict_error(task_id: &str, conflicts: &[FileLockConflict]) -> OrbitError {
    let details = conflicts
        .iter()
        .map(|conflict| {
            format!(
                "{} (held by {})",
                conflict.file_path, conflict.held_by_task_id
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    OrbitError::PolicyDenied(format!(
        "task '{task_id}' cannot acquire file locks because the following paths are already locked: {details}"
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use rusqlite::Connection;

    use crate::{FileLockChecker, apply_lock_schema};

    use super::FileLockStore;

    fn store() -> FileLockStore {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        apply_lock_schema(&conn).expect("schema");
        FileLockStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn acquires_and_releases_locks() {
        let store = store();
        store
            .acquire_locks("T1", "/repo", &["src/lib.rs", "src/main.rs"])
            .expect("acquire");

        let holder = store
            .lock_holder("/repo", "src/lib.rs")
            .expect("holder")
            .expect("present");
        assert_eq!(holder.task_id, "T1");

        let released = store.release_locks_for_task("T1").expect("release");
        assert_eq!(released, 2);
        assert!(
            store
                .lock_holder("/repo", "src/lib.rs")
                .expect("holder")
                .is_none(),
            "locks should be removed after release"
        );
    }

    #[test]
    fn detects_conflicts_from_other_task() {
        let store = store();
        store
            .acquire_locks("T1", "/repo", &["src/lib.rs"])
            .expect("acquire");

        let conflicts = store
            .check_conflicts("/repo", &["src/lib.rs", "src/other.rs"], Some("T2"))
            .expect("conflicts");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].held_by_task_id, "T1");

        let err = store
            .acquire_locks("T2", "/repo", &["src/lib.rs"])
            .expect_err("should conflict");
        assert!(err.to_string().contains("already locked"));
    }

    #[test]
    fn auto_acquire_is_idempotent_for_same_task() {
        let store = store();
        store
            .auto_acquire("T1", "/repo", "src/lib.rs")
            .expect("first acquire");
        store
            .auto_acquire("T1", "/repo", "src/lib.rs")
            .expect("second acquire");

        store
            .check_write_allowed(Some("T1"), "/repo", "src/lib.rs")
            .expect("same task should be allowed");
    }

    #[test]
    fn write_is_denied_when_other_task_holds_lock() {
        let store = store();
        store
            .auto_acquire("T1", "/repo", "src/lib.rs")
            .expect("acquire");

        let err = store
            .check_write_allowed(Some("T2"), "/repo", "src/lib.rs")
            .expect_err("other task should be denied");
        assert!(err.to_string().contains("locked by task 'T1'"));
    }

    #[test]
    fn stale_cleanup_removes_inactive_locks() {
        let store = store();
        store
            .acquire_locks("T1", "/repo", &["src/lib.rs"])
            .expect("acquire");
        store
            .acquire_locks("T2", "/repo", &["src/main.rs"])
            .expect("acquire");

        let removed = store.release_stale_locks(&["T2"]).expect("cleanup");
        assert_eq!(removed, 1);
        assert!(
            store
                .lock_holder("/repo", "src/lib.rs")
                .expect("holder")
                .is_none()
        );
        assert!(
            store
                .lock_holder("/repo", "src/main.rs")
                .expect("holder")
                .is_some()
        );
    }
}
