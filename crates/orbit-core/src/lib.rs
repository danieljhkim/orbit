#![deny(clippy::print_stderr, clippy::print_stdout)]
// Legacy runtime command surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Directional application operations, runtime mechanisms, adapters,
//! bootstrap, and composition.
//!
//! This is the library crate that assembles all subsystems into the
//! [`OrbitRuntime`] — the single entry point used by the CLI, Web, and
//! `orbit-cmd` adapters. Composition loads resolved config and joins bootstrap,
//! application operations, adapters, and the runtime kernel.
//!
//! # Role
//! Depends on the lower Orbit crates (never on `orbit-cmd`). Consumed by
//! `orbit-cmd`, `orbit-cli`, and `orbit-web`; neutral
//! kernels below this layer do not import from `orbit-core`.
//!
//! Shared use cases live in [`application`]. Tool-host and engine-host protocol
//! translation lives in [`adapter`]. Runtime code owns mechanisms and imports
//! neither application nor adapter modules.
//!
//! # Root re-export policy (ORB-10016)
//! Every root `pub use` below is justified by a real import in a consumer
//! crate (`orbit-cli`, `orbit-web`, `orbit-cmd`). Anything else must be
//! imported from its owning module (`orbit_core::application::…`,
//! `orbit_core::runtime::…`) or its owning crate (`orbit_common`,
//! `orbit_store`, `orbit_engine`).
//!
//! # Key exports
//! - [`OrbitRuntime`] — fully initialized runtime; wraps stores, policy, tools, and event bus
//! - [`ActorIdentity`] — actor identity for audit trail attribution
//! - [`OrbitError`] — re-exported from `orbit-common::types` for CLI-layer convenience
//! - `application::*` — coordinated use cases and their DTOs
//! - `adapter::*` — command, tool-host, and engine-host protocol translation
//! - `skill_catalog` — re-exported skill store for CLI skill lookup
//!
//! # Dependency direction
//! orbit-common, orbit-store, orbit-policy, orbit-tools, orbit-search, orbit-engine
//! → `orbit-core` → orbit-cmd / orbit-web / orbit-cli

pub mod adapter;
pub mod application;
pub mod auto_tasks;
pub mod bootstrap;
pub mod composition;
pub mod context;
pub mod metrics;
mod paths;
pub mod routines;
pub mod runtime;

// Store metric/scoreboard projections consumed by the dashboard's JSON API.
pub use orbit_store::scoreboard_summary;
pub use orbit_store::skill_store as skill_catalog;
pub use orbit_store::{
    ActivityInvocationMetrics, InvocationInsertParams, InvocationQuery, InvocationRecord,
    TaskInvocationMetrics, ToolInvocationMetrics,
};
pub use orbit_tools::prepare_remote_task_artifact_put;

// Command-layer types the CLI names in its clap surfaces.
pub use application::docs::{DocType, TaskRelatedDoc};
pub use application::job::{PipelineInvokeResult, PipelineWaitEntry};
pub use application::search::{
    GlobalSearchHit, GlobalSearchKind, GlobalSearchParams, task_selectors_contain_path,
};
pub use application::workflow::{ShipMode, build_ship_input, find_workflow, resolved_ship_mode};
pub use context::ActorIdentity;
// Shared domain types (owned by orbit-common) that the CLI and dashboard
// render or construct.
pub use auto_tasks::{AutoTaskAddParams, AutoTaskUpdateParams};
pub use orbit_common::security::redaction::redact_sensitive_env_text;
pub use orbit_common::{NotFoundKind, OrbitError};
pub use orbit_store::{
    AuditEventFilter, AuditEventInsertParams, AuditToolAggregate, V2AuditEventFilter,
    V2AuditEventInsertParams,
};
pub use orbit_types::task::{
    DEFAULT_TASK_LIST_LIMIT, ExternalRef, Task, TaskComplexity, TaskCreateStatus, TaskPriority,
    TaskStatus, TaskType, resolve_task_dependencies, resolve_task_relations,
    task_dependencies_ready,
};
pub use orbit_types::telemetry::{AuditEvent, AuditEventStatus, AuditStats};
pub use orbit_types::workflow::{
    AutoTaskDefinition, AutoTaskSchedule, AutoTaskTemplate, DedupePolicy, ExecutorDef, JobRun,
    JobRunState, JobRunStep, JobTargetType,
};
pub use orbit_types::workflow::{MissedRunPolicy, OverlapPolicy};
// Failure-incident grouping over the raw audit rows [ORB-10871]; consumed by
// the dashboard's incident, audit-summary, and scoreboard surfaces.
pub use orbit_store::{
    CASCADE_WINDOW_SECS, FailureClass, FailureIncident, FailureIncidentQuery,
    FailureIncidentReport, IncidentEventRef, PropagationLink,
};
// Routine fire records surfaced by the dashboard's routine-health JSON API.
pub use orbit_store::{RoutineFireRecord, RoutineFireState};
pub use runtime::engine::ResolvedCrewProjection;
pub use runtime::engine::{
    OrchestratorInvocationMetrics, OrchestratorInvocationMetricsBucket,
    OrchestratorMetricsBucketKind,
};
pub use runtime::{OrbitRuntime, WorkspaceRootHint, WorkspaceRuntimeBinding};
