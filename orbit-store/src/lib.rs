//! File-based (YAML) and SQLite persistence backends for Orbit data.
//!
//! Provides two storage backends — a file store for human-readable, git-friendly
//! YAML artifacts (tasks, jobs, activities, skills) and a SQLite store for
//! append-only data (audit events, stored tools). A layered store pattern merges
//! global and workspace-local stores with configurable scoping strategies.
//!
//! # Role
//! Depends only on `orbit-types`. Consumed by `orbit-core`, which constructs
//! the appropriate backend(s) and injects them into the [`OrbitRuntime`].
//!
//! # Key exports
//! - Backend trait types: [`TaskStoreBackend`], [`JobStoreBackend`], [`ActivityStoreBackend`],
//!   [`AuditEventStoreBackend`], [`ToolStoreBackend`]
//! - Factory functions: `task_store_file`, `task_store_resolved`, `job_store_file`,
//!   `job_store_resolved`, `activity_store_file`, `activity_store_resolved`,
//!   `audit_event_store_sqlite`, `tool_store_sqlite`
//! - [`ResolvedScope`] / [`ScopeResolution`] — scoping strategies (WorkspaceOnly, MergeByKey, etc.)
//! - [`Store`] / [`StoreTx`] — SQLite connection handle and transaction wrapper
//! - [`validate_instance_against_schema`] — JSON Schema validation for activity I/O
//!
//! # Dependency direction
//! `orbit-types` → `orbit-store` → orbit-core

pub mod backend;
mod file;
pub mod json_schema;
pub mod sqlite;

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

pub mod friction_bounty {
    pub use crate::file::friction_bounty::{
        record_friction_accepted, record_friction_rejected, record_friction_reported,
    };
}

pub mod friction_log {
    pub use crate::file::friction_log::{append_friction_entry, read_friction_entries_for_month};
}

pub mod metrics_log {
    pub use crate::file::metrics_log::{append_metrics_entry, read_metrics_entries_for_month};
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, AuditEventStoreBackend,
    JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams,
    ResolvedScope, ScopeResolution, TaskCreateParams, TaskStoreBackend, TaskUpdateParams,
    ToolStoreBackend, activity_store_file, activity_store_resolved, audit_event_store_sqlite,
    job_store_file, job_store_resolved, task_store_file, task_store_resolved, tool_store_sqlite,
};
pub use json_schema::{validate_instance_against_schema, validate_schema_document};
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
