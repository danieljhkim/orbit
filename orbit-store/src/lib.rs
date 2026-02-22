use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use orbit_types::{Audit, Job, JobStatus, Memo, OrbitError, OrbitEvent, Task, Watch};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value;

const GLOBAL_JOB_LOCK: &str = "jobs/global";

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

pub struct StoreTx<'a> {
    tx: Transaction<'a>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, OrbitError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OrbitError::Store(e.to_string()))?;
        }

        let conn = Connection::open(path).map_err(|e| OrbitError::Store(e.to_string()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, OrbitError> {
        let conn = Connection::open_in_memory().map_err(|e| OrbitError::Store(e.to_string()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS memos (
                id TEXT PRIMARY KEY,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                next_run_at TEXT NOT NULL,
                last_run_at TEXT,
                status TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS watches (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                command TEXT NOT NULL,
                debounce_ms INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS locks (
                name TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                acquired_at TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(())
    }

    pub fn with_transaction<T, F>(&self, op: F) -> Result<T, OrbitError>
    where
        F: FnOnce(&mut StoreTx<'_>) -> Result<T, OrbitError>,
    {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let tx = conn
            .transaction()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut store_tx = StoreTx { tx };
        let result = op(&mut store_tx)?;
        store_tx
            .tx
            .commit()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(result)
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT id, title, created_at FROM tasks ORDER BY created_at DESC")
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let created_at: String = row.get(2)?;
                Ok(Task {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: parse_timestamp(&created_at)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_memos(&self) -> Result<Vec<Memo>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT id, body, created_at FROM memos ORDER BY created_at DESC")
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let created_at: String = row.get(2)?;
                Ok(Memo {
                    id: row.get(0)?,
                    body: row.get(1)?,
                    created_at: parse_timestamp(&created_at)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_watches(&self) -> Result<Vec<Watch>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, path, command, debounce_ms, updated_at FROM watches ORDER BY updated_at DESC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let updated_at: String = row.get(4)?;
                let debounce_ms: i64 = row.get(3)?;
                Ok(Watch {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    command: row.get(2)?,
                    debounce_ms: debounce_ms as u64,
                    updated_at: parse_timestamp(&updated_at)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, event_type, payload, message, created_at FROM audits ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let payload_raw: String = row.get(2)?;
                let created_at_raw: String = row.get(4)?;

                let payload: Value = serde_json::from_str(&payload_raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        payload_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                Ok(Audit {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    payload,
                    message: row.get(3)?,
                    created_at: parse_timestamp(&created_at_raw)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn try_lock(&self, name: &str) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let changed = conn
            .execute(
                "INSERT INTO locks(name, owner, acquired_at) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO NOTHING",
                params![name, "orbit-core", now_string()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(changed == 1)
    }

    pub fn unlock(&self, name: &str) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let changed = conn
            .execute("DELETE FROM locks WHERE name = ?1", [name])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, command, next_run_at, last_run_at, status FROM jobs WHERE next_run_at <= ?1 AND status = 'scheduled' ORDER BY next_run_at ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([now.to_rfc3339()], |row| {
                let next_run_raw: String = row.get(3)?;
                let last_run_raw: Option<String> = row.get(4)?;
                let status_raw: String = row.get(5)?;
                Ok(Job {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    command: row.get(2)?,
                    next_run_at: parse_timestamp(&next_run_raw)?,
                    last_run_at: match last_run_raw {
                        Some(v) => Some(parse_timestamp(&v)?),
                        None => None,
                    },
                    status: str_to_status(&status_raw),
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn global_job_lock_name() -> &'static str {
        GLOBAL_JOB_LOCK
    }

    pub fn get_job_status(&self, id: &str) -> Result<Option<JobStatus>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let status = conn
            .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |row| {
                let raw: String = row.get(0)?;
                Ok(str_to_status(&raw))
            })
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(status)
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_task(&mut self, title: &str) -> Result<Task, OrbitError> {
        let task = Task {
            id: new_id("task"),
            title: title.to_string(),
            created_at: Utc::now(),
        };

        self.tx
            .execute(
                "INSERT INTO tasks(id, title, created_at) VALUES (?1, ?2, ?3)",
                params![task.id, task.title, task.created_at.to_rfc3339()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(task)
    }

    pub fn transition_job_status(
        &mut self,
        id: &str,
        from: JobStatus,
        to: JobStatus,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .tx
            .execute(
                "UPDATE jobs SET status = ?1 WHERE id = ?2 AND status = ?3",
                params![status_to_str(&to), id, status_to_str(&from)],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn complete_job(
        &mut self,
        id: &str,
        next_run_at: DateTime<Utc>,
        success: bool,
    ) -> Result<bool, OrbitError> {
        let final_state = if success {
            JobStatus::Complete
        } else {
            JobStatus::Failed
        };

        let changed = self
            .tx
            .execute(
                "UPDATE jobs SET status = ?1, last_run_at = ?2, next_run_at = ?3 WHERE id = ?4",
                params![
                    status_to_str(&final_state),
                    now_string(),
                    next_run_at.to_rfc3339(),
                    id
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(changed == 1)
    }

    pub fn insert_job(
        &mut self,
        name: &str,
        command: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<Job, OrbitError> {
        let job = Job {
            id: new_id("job"),
            name: name.to_string(),
            command: command.to_string(),
            next_run_at,
            last_run_at: None,
            status: JobStatus::Scheduled,
        };

        self.tx
            .execute(
                "INSERT INTO jobs(id, name, command, next_run_at, last_run_at, status) VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![job.id, job.name, job.command, job.next_run_at.to_rfc3339(), status_to_str(&job.status)],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(job)
    }

    pub fn insert_audit_event(&mut self, event: &OrbitEvent) -> Result<(), OrbitError> {
        let payload = serde_json::to_string(event).map_err(|e| OrbitError::Store(e.to_string()))?;
        let event_type = event_type(event);
        let message = event_message(event);
        self.tx
            .execute(
                "INSERT INTO audits(event_type, payload, message, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![event_type, payload, message, now_string()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(())
    }
}

fn event_type(event: &OrbitEvent) -> &'static str {
    match event {
        OrbitEvent::ToolExecuted { .. } => "ToolExecuted",
        OrbitEvent::JobStarted { .. } => "JobStarted",
        OrbitEvent::JobCompleted { .. } => "JobCompleted",
        OrbitEvent::WatchTriggered { .. } => "WatchTriggered",
        OrbitEvent::PolicyDenied { .. } => "PolicyDenied",
        OrbitEvent::TaskAdded { .. } => "TaskAdded",
    }
}

fn event_message(event: &OrbitEvent) -> String {
    match event {
        OrbitEvent::ToolExecuted { name } => format!("tool executed: {name}"),
        OrbitEvent::JobStarted { id } => format!("job started: {id}"),
        OrbitEvent::JobCompleted { id, success } => {
            format!("job completed: {id} (success={success})")
        }
        OrbitEvent::WatchTriggered { path } => format!("watch triggered: {path}"),
        OrbitEvent::PolicyDenied { tool } => format!("policy denied: {tool}"),
        OrbitEvent::TaskAdded { id } => format!("task added: {id}"),
    }
}

fn status_to_str(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Scheduled => "scheduled",
        JobStatus::Running => "running",
        JobStatus::Complete => "complete",
        JobStatus::Failed => "failed",
    }
}

fn str_to_status(raw: &str) -> JobStatus {
    match raw {
        "scheduled" => JobStatus::Scheduled,
        "running" => JobStatus::Running,
        "complete" => JobStatus::Complete,
        "failed" => JobStatus::Failed,
        _ => JobStatus::Failed,
    }
}

fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(parsed.with_timezone(&Utc))
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

fn new_id(prefix: &str) -> String {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{prefix}-{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_advisory_and_exclusive() {
        let store = Store::open_in_memory().expect("store");

        assert!(store.try_lock("abc").expect("first lock"));
        assert!(!store.try_lock("abc").expect("second lock fails"));
        assert!(store.unlock("abc").expect("unlock"));
        assert!(store.try_lock("abc").expect("lock again"));
    }

    #[test]
    fn mutation_persists_task_and_audit() {
        let store = Store::open_in_memory().expect("store");

        let task = store
            .with_transaction(|tx| {
                let task = tx.insert_task("buy milk")?;
                tx.insert_audit_event(&OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                })?;
                Ok(task)
            })
            .expect("mutation succeeds");

        let tasks = store.list_tasks().expect("list tasks");
        let audits = store.list_audits(10).expect("list audits");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "buy milk");
        assert_eq!(task.title, "buy milk");

        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "TaskAdded");
    }
}
