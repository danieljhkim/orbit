#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy runtime command surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Runtime bootstrap, config layering, command dispatch, and default asset seeding.
//!
//! This is the top-level library crate that assembles all subsystems into the
//! [`OrbitRuntime`] — the single entry point used by both the CLI and tests.
//! It handles initialization from disk (two-root layout: global + workspace),
//! config loading and merging, command execution, and default asset seeding via
//! embedded YAML templates.
//!
//! # Role
//! Depends on all other Orbit crates. Consumed only by `orbit-cli` (and tests).
//! Nothing below this layer should import from `orbit-core`.
//!
//! # Key exports
//! - [`OrbitRuntime`] — fully initialized runtime; wraps stores, policy, tools, and event bus
//! - [`OrbitContext`] — runtime context: stores, config, policy, tool registry
//! - [`ActorIdentity`] / [`ActorKind`] — actor identity for audit trail attribution
//! - [`OrbitError`] — re-exported from `orbit-common::types` for CLI-layer convenience
//! - `command::*` — command implementations (task, job, activity, skill, audit, tool, init)
//! - `skill_catalog` — re-exported skill store for CLI skill lookup
//!
//! # Dependency direction
//! orbit-common, orbit-policy, orbit-exec, orbit-tools, orbit-store, orbit-agent, orbit-engine
//! → `orbit-core` → orbit-cli

pub mod command;
pub mod config;
pub mod context;
pub mod knowledge_stats;
mod paths;
pub mod runtime;
pub mod workspace_registry;

pub use orbit_engine::JobRunResult;
pub use orbit_store::duel_scoreboard;
pub use orbit_store::scoreboard_summary;
pub use orbit_store::skill_store as skill_catalog;
pub use orbit_store::{
    ActivityInvocationMetrics, InvocationQuery, InvocationRecord, InvocationToolCallRecord,
    TaskInvocationMetrics, ToolInvocationMetrics,
};

pub use command::learning::migrate_learning_layout_at;
pub use command::task_template::TaskTemplate;
pub use command::workflow::{
    WORKFLOWS, Workflow, WorkflowInput, build_workflow_input, build_workflow_input_for,
    find_workflow, validate_workflow_flags,
};
pub use context::{ActorIdentity, ActorKind, OrbitContext};
pub use orbit_common::types::{
    Activity, AuditEvent, AuditEventStatus, AuditStats, EvidenceKind, ExecutorDef, ExternalRef,
    GITHUB_PR_EXTERNAL_REF_SYSTEM, Job, JobRun, JobRunState, JobRunStep, JobScheduleState, JobStep,
    JobTargetType, Learning, LearningEvidence, LearningScope, LearningStatus,
    ResolvedTaskDependency, ReviewMessage, ReviewThread, ReviewThreadStatus, Role, Skill, Task,
    TaskComment, TaskComplexity, TaskPriority, TaskStatus, TaskType, build_task_status_index,
    normalize_task_dependencies, normalize_task_tags, push_external_ref_if_missing,
    resolve_task_dependencies, task_dependencies_ready, task_matches_tags, unmet_task_dependencies,
    validate_task_dependencies,
};
pub use orbit_common::types::{NotFoundKind, OrbitError};
pub use orbit_common::utility::redaction::{
    redact_sensitive_env_error, redact_sensitive_env_json, redact_sensitive_env_option,
    redact_sensitive_env_text,
};
pub use orbit_store::AuditEventInsertParams;
pub use orbit_store::learning_layout::LearningLayoutMigrationReport;
pub use orbit_store::{
    LearningCreateParams, LearningSearchParams, LearningSearchResult, LearningUpdateParams,
};
pub use runtime::OrbitRuntime;
pub use runtime::engine::{
    ConfiguredCrewProjection, ConfiguredCrewRegistryProjection, ResolvedCrewProjection,
};
