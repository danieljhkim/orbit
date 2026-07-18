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
//! Runtime bootstrap, config layering, runtime-integrated command dispatch,
//! and default asset seeding.
//!
//! This is the library crate that assembles all subsystems into the
//! [`OrbitRuntime`] — the single entry point used by the CLI, the dashboard,
//! and the extracted `orbit-cmd` command layer. It handles initialization
//! from disk (two-root layout: global + workspace), config loading and
//! merging, and default asset seeding via embedded YAML templates.
//!
//! # Role
//! Depends on the lower Orbit crates (never on `orbit-cmd`). Consumed by
//! `orbit-cmd`, `orbit-cli`, and `orbit-dashboard`; nothing below this layer
//! imports from `orbit-core`.
//!
//! Command groups that runtime internals invoke (tool hosts, engine hosts,
//! bootstrap seeding) live in [`command`]; CLI-only command groups were
//! extracted to `orbit-cmd` in [ORB-10016].
//!
//! # Root re-export policy (ORB-10016)
//! Every root `pub use` below is justified by a real import in a consumer
//! crate (`orbit-cli`, `orbit-dashboard`, `orbit-cmd`). Anything else must be
//! imported from its owning module (`orbit_core::command::…`,
//! `orbit_core::runtime::…`) or its owning crate (`orbit_common`,
//! `orbit_store`, `orbit_engine`).
//!
//! # Key exports
//! - [`OrbitRuntime`] — fully initialized runtime; wraps stores, policy, tools, and event bus
//! - [`ActorIdentity`] — actor identity for audit trail attribution
//! - [`OrbitError`] — re-exported from `orbit-common::types` for CLI-layer convenience
//! - `command::*` — runtime-integrated command implementations
//! - `skill_catalog` — re-exported skill store for CLI skill lookup
//!
//! # Dependency direction
//! orbit-common, orbit-policy, orbit-exec, orbit-tools, orbit-store, orbit-agent, orbit-engine
//! → `orbit-core` → orbit-cmd → orbit-cli / orbit-dashboard

pub mod auto_tasks;
pub mod command;
pub mod config;
pub mod context;
pub mod host_registry;
pub mod metrics;
mod paths;
pub mod routines;
pub mod runtime;
pub mod workspace_registry;

// Store metric/scoreboard projections consumed by the dashboard's JSON API.
pub use orbit_store::scoreboard_summary;
pub use orbit_store::skill_store as skill_catalog;
pub use orbit_store::{
    ActivityInvocationMetrics, InvocationInsertParams, InvocationQuery, InvocationRecord,
    TaskInvocationMetrics, ToolInvocationMetrics,
};
// Canonical builtin MCP definitions are re-exported for the CLI without requiring
// an OrbitRuntime or adding a new CLI -> orbit-tools dependency edge.
pub use orbit_tools::canonical_builtin_mcp_tool_definitions;

// Command-layer types the CLI names in its clap surfaces.
pub use command::docs::{DocType, TaskRelatedDoc};
pub use command::learning::migrate_learning_layout_at;
pub use command::search::{
    GlobalSearchHit, GlobalSearchKind, GlobalSearchParams, task_selectors_contain_path,
};
pub use command::workflow::{ShipMode, build_ship_input, find_workflow, resolved_ship_mode};
pub use context::ActorIdentity;
pub use host_registry::HostRegistryService;
// Shared domain types (owned by orbit-common) that the CLI and dashboard
// render or construct.
pub use auto_tasks::{AutoTaskAddParams, AutoTaskUpdateParams};
pub use orbit_common::types::{
    AuditEvent, AuditEventStatus, AuditStats, AutoTaskDefinition, AutoTaskSchedule,
    AutoTaskTemplate, DedupePolicy, EvidenceKind, ExecutionProfileCrewV1, ExecutionProfileShipV1,
    ExecutionProfileV1, ExecutorDef, ExternalRef, HostAlias, HostNameResolution, HostRecord,
    HostRegistration, HostStatus, HostWorkspacePresence, JobRun, JobRunState, JobRunStep,
    JobTargetType, Learning, LearningEvidence, LearningScope, LearningStatus, ProjectionFreshness,
    ReviewThreadStatus, SanitizedExecutionProfile, SanitizedWorkspacePresence,
    StoredExecutionProfile, Task, TaskComplexity, TaskCreateStatus, TaskPriority, TaskStatus,
    TaskType, WorkspaceOwnership, WorkspacePresenceDeclaration, build_task_status_index,
    resolve_task_dependencies, task_dependencies_ready,
};
pub use orbit_common::types::{MissedRunPolicy, NotFoundKind, OrbitError, OverlapPolicy};
pub use orbit_common::utility::redaction::redact_sensitive_env_text;
pub use orbit_store::learning_layout::LearningLayoutMigrationReport;
pub use orbit_store::{
    AuditEventFilter, AuditEventInsertParams, AuditToolAggregate, V2AuditEventFilter,
    V2AuditEventInsertParams,
};
pub use orbit_store::{
    LearningCreateParams, LearningListEntry, LearningSearchParams, LearningUpdateParams,
};
// Routine fire records surfaced by the dashboard's routine-health JSON API.
pub use orbit_store::{RoutineFireRecord, RoutineFireState};
pub use runtime::OrbitRuntime;
pub use runtime::engine::ResolvedCrewProjection;
