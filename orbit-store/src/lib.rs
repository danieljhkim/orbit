pub mod backend;
mod file;
pub mod sqlite;

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, AuditEventStoreBackend,
    JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams,
    LockStoreBackend, TaskCreateParams, TaskStoreBackend, TaskUpdateParams, ToolStoreBackend,
    activity_store_file, activity_store_layered, activity_store_memory, audit_event_store_sqlite,
    job_store_file, job_store_layered, job_store_memory, lock_store_memory, task_store_file,
    task_store_memory, tool_store_sqlite,
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
