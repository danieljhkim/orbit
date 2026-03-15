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
    AuditEventStoreBackend, JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend,
    JobUpdateParams, LockStoreBackend, TaskCreateParams, TaskStoreBackend, TaskUpdateParams,
    ToolStoreBackend, activity_store_file, agent_session_store_sqlite, audit_event_store_sqlite,
    job_store_file, lock_store_memory, task_store_file, tool_store_sqlite,
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
mod tests {}
