#![deny(clippy::print_stderr, clippy::print_stdout)]
//! File-based (YAML) and SQLite persistence backends for Orbit data.
//!
//! Provides two storage backends — a file store for human-readable, git-friendly
//! YAML artifacts (tasks, jobs, activities, skills) and a SQLite store for
//! append-only data (audit events, stored tools). Store builders make the
//! supported workspace/global split explicit per domain.
//!
//! # Role
//! Depends only on `orbit-types`. Consumed by `orbit-core`, which constructs
//! the appropriate backend(s) and injects them into the [`OrbitRuntime`].
//!
//! # Key exports
//! - Backend trait types: [`TaskStoreBackend`], [`TaskDocumentStoreBackend`],
//!   [`TaskHistoryStoreBackend`], [`TaskReviewStoreBackend`],
//!   [`TaskArtifactStoreBackend`], [`TaskReservationStoreBackend`],
//!   [`JobRunStoreBackend`], [`AuditEventStoreBackend`], [`ToolStoreBackend`]
//! - Factory functions: `workspace_task_backends`, `workspace_job_run_store`,
//!   `global_executor_def_store`, `global_policy_def_store`,
//!   `audit_event_store_sqlite`, `task_reservation_store_sqlite`, `tool_store_sqlite`
//! - [`Store`] / [`StoreTx`] — SQLite connection handle and transaction wrapper
//! - [`validate_instance_against_schema`] — JSON Schema validation for activity I/O
//!
//! # Dependency direction
//! `orbit-types` → `orbit-store` → orbit-core

pub mod backend;
mod file;
#[path = "sqlite/invocation_store.rs"]
mod invocation_store_impl;
pub mod json_schema;
pub mod scope;
pub mod sqlite;
pub mod state_io;

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

pub mod friction_store {
    pub use crate::file::friction_store::{
        FrictionAddParams, FrictionListFilter, StoredFrictionRecord, add_friction,
        ensure_default_tag_taxonomy, friction_stats, list_frictions, show_friction,
    };
}

pub mod pr_scoreboard {
    pub use crate::file::scoreboard::pr_scoreboard::{
        record_pr_count_with_revision, record_pr_count_without_revision, record_pr_review_comment,
    };
}

pub mod task_review_scoreboard {
    pub use crate::file::scoreboard::task_review_scoreboard::record_task_review_thread;
}

pub mod scoreboard_summary {
    pub use crate::file::scoreboard::scoreboard_summary::{
        AgentSummary, DuelSummary, PrSummary, RecentSummary, ScoreboardInputs, ScoreboardSummary,
        TaskReviewSummary, TokenSummary, TopToolCall, WorkflowRunCount, generate_summary,
        generate_summary_with_audit_tool_calls, generate_summary_with_inputs, summary_path,
        write_summary,
    };
}

pub mod duel_scoreboard {
    pub use crate::file::scoreboard::duel_scoreboard::{
        AggregateFilter, AggregateRow, Aggregates, ReviewerTally, RoleAxis, SegmentBy, aggregate,
        append_run, derive_task_scope, known_agent_families, load_runs, tally_reviewer_stats,
    };
}

pub mod planning_duel_scoreboard {
    pub use crate::file::scoreboard::planning_duel_scoreboard::{
        AggregateFilter, AggregateRow, Aggregates, RoleAxis, aggregate, append_run, load_runs,
    };
}

pub mod friction_log {
    pub use crate::file::diagnostics::friction_log::{
        append_friction_entry, read_friction_entries_for_month,
    };
}

pub mod metrics_log {
    pub use crate::file::diagnostics::metrics_log::{
        append_metrics_entry, read_metrics_entries_for_month,
    };
}

pub mod token_scoreboard {
    pub use crate::file::scoreboard::token_scoreboard::write_token_scoreboard;
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActiveTaskReservation, AdrCreateParams, AdrDocumentUpdateParams, AdrStoreBackend,
    AuditEventStoreBackend, ExecutorDefStoreBackend, ExpiredTaskReservation, JobRunQuery,
    JobRunStepParams, JobRunStoreBackend, LearningCreateParams, LearningSearchParams,
    LearningSearchResult, LearningStoreBackend, LearningUpdateParams, PolicyDefStoreBackend,
    ReleasedTaskReservation, TaskArtifactStoreBackend, TaskArtifactUpdateParams, TaskCreateParams,
    TaskDocumentStoreBackend, TaskDocumentUpdateParams, TaskHistoryStoreBackend,
    TaskHistoryUpdateParams, TaskLockConflict, TaskLockHolder, TaskReservationCheckParams,
    TaskReservationCheckResult, TaskReservationListResult, TaskReservationOwnedConflictsParams,
    TaskReservationOwnedConflictsResult, TaskReservationReleaseByOwnerParams,
    TaskReservationReleaseByOwnerResult, TaskReservationReleaseParams,
    TaskReservationReleaseReason, TaskReservationReleaseResult, TaskReservationReserveParams,
    TaskReservationReserveResult, TaskReservationStoreBackend, TaskReviewStoreBackend,
    TaskReviewUpdateParams, TaskStoreBackend, ToolStoreBackend, WorkspaceTaskBackends,
    audit_event_store_sqlite, global_executor_def_store, global_policy_def_store,
    layered_policy_def_store, task_reservation_store_sqlite, tool_store_sqlite,
    workspace_adr_backends, workspace_job_run_store, workspace_learning_backend,
    workspace_policy_def_store, workspace_task_backends,
};
pub use invocation_store_impl::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationInsertParams, InvocationQuery,
    InvocationRecord, InvocationToolCallRecord, TaskInvocationMetrics, ToolInvocationMetrics,
};
pub use json_schema::{validate_instance_against_schema, validate_schema_document};
pub use sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditToolCallCountsByRole,
    AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall,
};
pub use sqlite::connection::{Store, StoreTx};

pub(crate) fn parse_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(parsed.with_timezone(&Utc))
}

pub(crate) fn now_string() -> String {
    Utc::now().to_rfc3339()
}
