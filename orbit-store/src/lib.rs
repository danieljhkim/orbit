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
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, AgentSessionStoreBackend,
    AuditEventStoreBackend, AuditStoreBackend, ClaimedJobRun, DueJobsClaim, JobCreateParams,
    JobRunCompletionParams, JobRunQuery, JobStoreBackend, LockStoreBackend, TaskCreateParams,
    TaskStoreBackend, TaskUpdateParams, ToolStoreBackend, activity_store_file,
    agent_session_store_sqlite, audit_event_store_sqlite, audit_store_sqlite, job_store_file,
    lock_store_sqlite, task_store_file, tool_store_sqlite,
};
pub use sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};
pub use sqlite::connection::{Store, StoreTx};

pub(crate) fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(parsed.with_timezone(&Utc))
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use orbit_types::OrbitEvent;

    use crate::Store;

    #[test]
    fn lock_is_advisory_and_exclusive() {
        let store = Store::open_in_memory().expect("store");

        assert!(store.try_lock("abc").expect("first lock"));
        assert!(!store.try_lock("abc").expect("second lock fails"));
        assert!(store.unlock("abc").expect("unlock"));
        assert!(store.try_lock("abc").expect("lock again"));
    }

    #[test]
    fn mutation_persists_audit() {
        let store = Store::open_in_memory().expect("store");

        store
            .with_transaction(|tx| {
                tx.insert_audit_event(&OrbitEvent::TaskAdded {
                    id: "task-test-1".to_string(),
                })?;
                Ok(())
            })
            .expect("mutation succeeds");

        let audits = store.list_audits(10).expect("list audits");

        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "TaskAdded");
    }
}
