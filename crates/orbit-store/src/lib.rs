#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy persistence surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! File-based (YAML) and SQLite persistence backends for Orbit data.
//!
//! Provides two storage backends — a file store for human-readable, git-friendly
//! YAML artifacts (tasks, jobs, activities, skills) and a SQLite store for
//! append-only data (audit events, stored tools). Store builders make the
//! supported workspace/global split explicit per domain. The SQLite layer also
//! provides generic connection/transaction primitives and a namespaced feature
//! migration ledger; feature crates own their active schemas and queries while
//! Store retains any immutable historical bootstrap migrations needed for compatibility.
//!
//! # Role
//! Depends only on `orbit-common`. Consumed by `orbit-core`, `orbit-engine`,
//! `orbit-cmd`, and vertical feature crates such as `orbit-remote`.
//!
//! # Key exports
//! - Backend trait types: [`TaskStoreBackend`], [`TaskDocumentStoreBackend`],
//!   [`TaskHistoryStoreBackend`],
//!   [`TaskArtifactStoreBackend`], [`TaskReservationStoreBackend`],
//!   [`JobRunStoreBackend`], [`AuditEventStoreBackend`], [`ToolStoreBackend`]
//! - Factory functions: `workspace_task_backends`, `workspace_job_run_store`,
//!   `global_executor_def_store`, `global_policy_def_store`,
//!   `audit_event_store_sqlite`, `task_reservation_store_sqlite`, `tool_store_sqlite`
//! - [`Store`] / [`StoreTx`] — SQLite connection handle and transaction wrapper
//! - [`validate_instance_against_schema`] — JSON Schema validation for activity I/O
//!
//! # Dependency direction
//! `orbit-common` ← `orbit-store` ← consumers such as orbit-core and orbit-remote

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

pub mod friction_store {
    pub use crate::file::friction_store::{
        FrictionAddParams, FrictionListFilter, FrictionUpdateParams, StoredFrictionRecord,
        add_friction, canonical_hub_friction_root, ensure_default_tag_taxonomy, friction_stats,
        friction_tags, list_frictions, prepare_hub_friction_root, readable_hub_friction_root,
        resolve_friction, resolve_friction_by_task, show_friction, update_friction,
        validate_friction_id,
    };
}

pub mod pr_scoreboard {
    pub use crate::file::scoreboard::pr_scoreboard::{
        record_pr_count_with_revision, record_pr_count_without_revision,
    };
}

pub mod scoreboard_summary {
    pub use crate::file::scoreboard::scoreboard_summary::{
        AgentSummary, FrictionSummary, KnowledgeSummary, NormalizedTokenSummary,
        ORCHESTRATION_SCHEMA_VERSION, OrchestrationBucketKind, OrchestrationBucketSummary,
        OrchestrationModelSummary, OrchestrationSummary, PrSummary, RecentSummary,
        ScoreboardInputs, ScoreboardSummary, ScoreboardWindow, TokenSummary, TopToolCall,
        WorkflowRunCount, generate_summary, generate_summary_with_audit_tool_calls,
        generate_summary_with_inputs, summary_path, write_summary,
    };
}

pub mod token_scoreboard {
    pub use crate::file::scoreboard::token_scoreboard::write_token_scoreboard;
}

pub mod learning_layout {
    pub use crate::file::learning_store::migration::{
        LearningLayoutMigrationReport, inspect_learning_layout, migrate_learning_layout,
    };
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActiveTaskReservation, AdrArtifact, AdrArtifactResolution, AdrCreateParams,
    AdrDocumentUpdateParams, AdrListEntry, AdrListFilter, AdrStoreBackend, AuditEventStoreBackend,
    ExecutorDefStoreBackend, ExpiredTaskReservation, JobRunQuery, JobRunStepParams,
    JobRunStoreBackend, LearningCreateParams, LearningListEntry, LearningSearchParams,
    LearningSearchResult, LearningStoreBackend, LearningUpdateParams, PolicyDefStoreBackend,
    ReleasedTaskReservation, RemoteArtifactStub, TaskArtifactStoreBackend,
    TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentStoreBackend, TaskDocumentUpdateParams,
    TaskHistoryStoreBackend, TaskHistoryUpdateParams, TaskLockConflict, TaskLockHolder,
    TaskReservationCheckParams, TaskReservationCheckResult, TaskReservationListResult,
    TaskReservationOwnedConflictsParams, TaskReservationOwnedConflictsResult,
    TaskReservationReleaseByOwnerParams, TaskReservationReleaseByOwnerResult,
    TaskReservationReleaseParams, TaskReservationReleaseReason, TaskReservationReleaseResult,
    TaskReservationReserveParams, TaskReservationReserveResult, TaskReservationStoreBackend,
    TaskStoreBackend, ToolStoreBackend, WorkspaceTaskBackends, audit_event_store_sqlite,
    coordination_task_backends, global_executor_def_store, global_policy_def_store,
    layered_policy_def_store, task_reservation_store_sqlite, tool_store_sqlite,
    workspace_adr_backends, workspace_job_run_store, workspace_learning_backend,
    workspace_policy_def_store, workspace_task_backends,
};
pub use file_lock::{LockHolderInfo, read_lock_holder};
pub use json_schema::{validate_instance_against_schema, validate_schema_document};
pub use sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditRoleAggregate, AuditToolAggregate,
    AuditToolCallCountsByRole, AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall,
    LEARNING_INJECTED_TARGET_TYPE, LEARNING_SHOWN_TARGET_TYPE, LearningUsageStat,
};
pub use sqlite::connection::{Store, StoreTx};
pub use sqlite::id_allocator::{
    IdAllocation, IdAllocationKind, IdAllocationRecord, IdAllocator, IdAllocatorConfig,
    LearningIdMigrationReport, LearningIdRename, ensure_id_allocation_schema,
    with_active_id_allocations,
};
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
