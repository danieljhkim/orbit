pub mod backend;
mod file;
pub mod sqlite;

pub mod identity_store {
    pub use crate::file::identity_store::*;
}

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

use chrono::{DateTime, Utc};

pub use backend::{
    AgentSessionStoreBackend, AuditEventStoreBackend, AuditStoreBackend, JobCreateParams,
    JobStoreBackend, LockStoreBackend, SchedulerCreateParams, SchedulerStoreBackend,
    TaskCreateParams, TaskStoreBackend, TaskUpdateParams, ToolStoreBackend, WatchStoreBackend,
    agent_session_store_sqlite, audit_event_store_sqlite, audit_store_sqlite, job_store_file,
    job_store_sqlite, lock_store_sqlite, scheduler_store_file, scheduler_store_sqlite,
    task_store_file, task_store_sqlite, tool_store_sqlite, watch_store_sqlite,
};
pub use sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};
pub use sqlite::connection::{Store, StoreTx};
pub use sqlite::job_store::JobInsertParams;
pub use sqlite::scheduler_store::{ClaimedJobRun, DueJobsClaim};
pub use sqlite::task_store;

pub(crate) fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(parsed.with_timezone(&Utc))
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn new_id(prefix: &str) -> String {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{prefix}-{nanos}")
}

#[cfg(test)]
mod tests {
    use orbit_types::OrbitEvent;

    use crate::Store;
    use crate::task_store::TaskInsertParams;

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
                let task = tx.insert_task(&TaskInsertParams {
                    title: "buy milk".to_string(),
                    ..Default::default()
                })?;
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
