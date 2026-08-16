#![deny(clippy::print_stderr, clippy::print_stdout)]
// Legacy persistence surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! File-based and SQLite persistence backends for Orbit data.
//!
//! Provides file stores for human-readable YAML and JSONL artifacts, plus a
//! SQLite store for relational and append-only data. Store builders make the
//! supported workspace/global split explicit per domain. The SQLite layer also
//! provides generic connection/transaction primitives and a namespaced feature
//! migration ledger; feature crates own their active schemas and queries while
//! Store retains any immutable historical bootstrap migrations needed for compatibility.
//!
//! # Role
//! Depends only on `orbit-common`. Consumed by `orbit-core`, `orbit-engine`,
//! and `orbit-cmd`.
//!
//! # Key exports
//! - Backend trait types: [`TaskStoreBackend`], [`TaskDocumentStoreBackend`],
//!   [`TaskHistoryStoreBackend`],
//!   [`TaskArtifactStoreBackend`], [`TaskReservationStoreBackend`],
//!   [`JobRunStoreBackend`], [`AuditEventStoreBackend`], [`ToolStoreBackend`]
//! - Factory functions: `workspace_task_backends`, `workspace_job_run_store`,
//!   `global_executor_def_store`, `global_policy_def_store`,
//!   `audit_event_store_sqlite`, `task_reservation_store_sqlite`, `tool_store_sqlite`
//! - [`SessionLogStore`] — lock-safe workspace session-log persistence
//! - [`Store`] / [`StoreTx`] — SQLite connection handle and transaction wrapper
//! - [`validate_instance_against_schema`] — JSON Schema validation for activity I/O
//!
//! # Dependency direction
//! `orbit-common` ← `orbit-store` ← consumers such as orbit-core and orbit-engine

pub(crate) mod backend;
mod file;
pub(crate) mod file_lock;
pub(crate) mod json_schema;
pub mod layout;
pub(crate) mod scope;
pub mod sqlite;
pub mod state_io;
pub mod task_migration;

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

/// Friction records. Live reads and writes go through [`FrictionStore`]
/// (SQLite, ORB-10680); the file-layout helpers re-exported here own the hub
/// publication decision and the legacy tree kept as read-only evidence.
pub mod friction_store {
    pub use orbit_types::identity::validate_friction_id;

    pub use crate::file::friction_store::{
        canonical_hub_friction_root, ensure_default_tag_taxonomy, prepare_hub_friction_root,
        readable_hub_friction_root,
    };
    pub use crate::sqlite::friction_store::{
        FrictionAddParams, FrictionImportReport, FrictionListFilter, FrictionReportedCount,
        FrictionStore, FrictionUpdateParams, StoredFrictionRecord, export_workspace_frictions,
    };
}

pub mod pr_scoreboard {
    pub use crate::file::scoreboard::pr_scoreboard::{
        record_pr_count_with_revision, record_pr_count_without_revision,
    };
}

pub mod scoreboard_summary {
    pub use crate::file::scoreboard::scoreboard_summary::{
        AgentSummary, CoverageAvailability, CoverageNote, FrictionSummary, NormalizedTokenSummary,
        NotableCompletion, NotableCompletions, ORCHESTRATION_SCHEMA_VERSION,
        OrchestrationBucketKind, OrchestrationBucketSummary, OrchestrationModelSummary,
        OrchestrationSummary, PrSummary, RecentSummary, ScoreboardCoverage, ScoreboardInputs,
        ScoreboardSummary, ScoreboardWindow, TokenSummary, TopToolCall, WorkflowRunCount,
        generate_summary, generate_summary_with_audit_tool_calls, generate_summary_with_inputs,
        summary_path, write_summary,
    };
}

pub mod token_scoreboard {
    pub use crate::file::scoreboard::token_scoreboard::write_token_scoreboard;
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActiveTaskReservation, AuditEventStoreBackend, ExecutorDefStoreBackend, ExpiredTaskReservation,
    JobRunQuery, JobRunStepParams, JobRunStoreBackend, PolicyDefStoreBackend,
    ReleasedTaskReservation, TaskArtifactStoreBackend, TaskArtifactUpdateParams, TaskCreateParams,
    TaskDocumentStoreBackend, TaskDocumentUpdateParams, TaskHistoryStoreBackend,
    TaskHistoryUpdateParams, TaskLockConflict, TaskLockHolder, TaskReservationCheckParams,
    TaskReservationCheckResult, TaskReservationListResult, TaskReservationOwnedConflictsParams,
    TaskReservationOwnedConflictsResult, TaskReservationReleaseByOwnerParams,
    TaskReservationReleaseByOwnerResult, TaskReservationReleaseParams,
    TaskReservationReleaseReason, TaskReservationReleaseResult, TaskReservationReserveParams,
    TaskReservationReserveResult, TaskReservationScope, TaskReservationStoreBackend,
    TaskStoreBackend, ToolStoreBackend, WorkspaceClaimAcquireParams, WorkspaceClaimAcquireResult,
    WorkspaceClaimCheckParams, WorkspaceClaimCheckResult, WorkspaceClaimHolder,
    WorkspaceClaimReleaseParams, WorkspaceClaimReleaseResult, WorkspaceClaimStatusResult,
    WorkspaceTaskBackends, audit_event_store_sqlite, coordination_task_backends,
    global_executor_def_store, global_policy_def_store, layered_policy_def_store,
    task_reservation_store_sqlite, tool_store_sqlite, workspace_job_run_store,
    workspace_policy_def_store, workspace_task_backends,
};
pub use file::session_log_store::{
    SessionLogAppendParams, SessionLogEntry, SessionLogFilter, SessionLogKind, SessionLogStore,
};
pub use file_lock::{LockHolderInfo, read_lock_holder};
pub use json_schema::{validate_instance_against_schema, validate_schema_document};
pub use sqlite::audit_event_store::incident::{
    CASCADE_WINDOW_SECS, DEFAULT_SCAN_LIMIT as FAILURE_INCIDENT_SCAN_LIMIT, FailureClass,
    FailureIncident, FailureIncidentQuery, FailureIncidentReport, IncidentEventRef,
    PropagationLink, build_report as build_failure_incident_report, classify as classify_failure,
    group_failure_incidents, normalize_message as normalize_failure_message,
    signature_for as failure_signature_for,
};
pub use sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditRoleAggregate, AuditToolAggregate,
    AuditToolCallCountsByRole, AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall,
};
pub use sqlite::connection::{Store, StoreTx};
pub use sqlite::invocation_store::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationAccountingFact,
    InvocationAccountingQuery, InvocationInsertParams, InvocationQuery, InvocationRecord,
    InvocationToolCallRecord, TaskInvocationMetrics, ToolInvocationMetrics,
};
pub use sqlite::reliability_store::{
    ActivityInvocationCount, BoundedFacts, InvocationRunCoverage, JobRunOutcomeFact,
};
pub use sqlite::routine_store::{
    RoutineCursor, RoutineFireIntentParams, RoutineFireRecord, RoutineFireState,
    RoutinePauseRecord, RoutineSweepLock, try_acquire_routine_sweep_lock,
};
pub use sqlite::task_registry::workspace_id_for_orbit_dir;
pub use sqlite::v2_audit_store::{V2AuditEventFilter, V2AuditEventInsertParams, V2AuditEventRow};

pub(crate) fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(parsed.with_timezone(&Utc))
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}
