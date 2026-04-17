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
//!   [`TaskArtifactStoreBackend`], [`JobDefinitionStoreBackend`],
//!   [`JobRunStoreBackend`], [`ActivityStoreBackend`], [`AuditEventStoreBackend`],
//!   [`ToolStoreBackend`]
//! - Factory functions: `workspace_task_backends`, `scoped_job_backends`,
//!   `global_activity_store`, `global_executor_def_store`,
//!   `global_policy_def_store`, `audit_event_store_sqlite`, `tool_store_sqlite`
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
pub mod sqlite;
pub mod state_io;
#[path = "file/token_scoreboard.rs"]
mod token_scoreboard_impl;

pub mod skill_store {
    pub use crate::file::skill_store::*;
}

pub mod friction_bounty {
    pub use crate::file::friction_bounty::{
        record_friction_accepted, record_friction_rejected, record_friction_reported,
    };
}

pub mod pr_scoreboard {
    pub use crate::file::pr_scoreboard::{
        record_pr_count_with_revision, record_pr_count_without_revision, record_pr_review_comment,
    };
}

pub mod scoreboard_summary {
    pub use crate::file::scoreboard_summary::{
        AgentSummary, DuelSummary, FrictionSummary, PrSummary, ScoreboardSummary, TokenSummary,
        generate_summary, summary_path, write_summary,
    };
}

pub mod duel_scoreboard {
    pub use crate::file::duel_scoreboard::{
        AggregateFilter, AggregateRow, Aggregates, ReviewerTally, RoleAxis, SegmentBy, aggregate,
        append_run, derive_task_scope, known_agent_families, load_runs, tally_reviewer_stats,
    };
}

pub mod planning_duel_scoreboard {
    pub use crate::file::planning_duel_scoreboard::{
        AggregateFilter, AggregateRow, Aggregates, RoleAxis, aggregate, append_run, load_runs,
    };
}

pub mod knowledge_stats {
    pub use crate::file::knowledge_stats::{
        DoubleReadSummary, KnowledgeStatsSummary, RatioSummary, TokenInputSummary, aggregate,
    };
}

pub mod friction_log {
    pub use crate::file::friction_log::{append_friction_entry, read_friction_entries_for_month};
}

pub mod metrics_log {
    pub use crate::file::metrics_log::{append_metrics_entry, read_metrics_entries_for_month};
}

pub mod token_scoreboard {
    pub use crate::token_scoreboard_impl::write_token_scoreboard;
}

use chrono::{DateTime, Utc};

pub use backend::{
    ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams, AuditEventStoreBackend,
    ExecutorDefStoreBackend, JobCreateParams, JobDefinitionStoreBackend, JobRunQuery,
    JobRunStepParams, JobRunStoreBackend, JobUpdateParams, PolicyDefStoreBackend,
    ScopedJobBackends, TaskArtifactStoreBackend, TaskArtifactUpdateParams, TaskCreateParams,
    TaskDocumentStoreBackend, TaskDocumentUpdateParams, TaskHistoryStoreBackend,
    TaskHistoryUpdateParams, TaskReviewStoreBackend, TaskReviewUpdateParams, TaskStoreBackend,
    ToolStoreBackend, WorkspaceTaskBackends, audit_event_store_sqlite, global_activity_store,
    global_executor_def_store, global_policy_def_store, scoped_job_backends, tool_store_sqlite,
    workspace_task_backends,
};
pub use invocation_store_impl::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationInsertParams, InvocationQuery,
    InvocationRecord, InvocationToolCallRecord, TaskInvocationMetrics, ToolInvocationMetrics,
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
