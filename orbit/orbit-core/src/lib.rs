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
//! - [`OrbitError`] — re-exported from `orbit-types` for CLI-layer convenience
//! - `command::*` — command implementations (task, job, activity, skill, audit, tool, init)
//! - `skill_catalog` — re-exported skill store for CLI skill lookup
//!
//! # Dependency direction
//! orbit-types, orbit-policy, orbit-exec, orbit-tools, orbit-store, orbit-agent, orbit-engine
//! → `orbit-core` → orbit-cli

pub mod command;
mod config;
pub mod context;
pub(crate) mod fs_utils;
mod paths;
pub mod runtime;
pub mod workspace_registry;

pub use orbit_engine::JobRunResult;
pub use orbit_store::skill_store as skill_catalog;

pub use command::task_template::TaskTemplate;
pub use command::workflow::{
    WORKFLOWS, Workflow, WorkflowInput, build_workflow_input, find_workflow,
    validate_workflow_flags,
};
pub use context::{ActorIdentity, ActorKind, OrbitContext};
pub use orbit_store::AuditEventInsertParams;
pub use orbit_types::OrbitError;
pub use orbit_types::{
    Activity, AuditEvent, AuditEventStatus, AuditStats, Job, JobRun, JobRunState, JobRunStep,
    JobScheduleState, JobStep, JobTargetType, ReviewMessage, ReviewThread, ReviewThreadStatus,
    Role, Skill, Task, TaskComment, TaskComplexity, TaskPriority, TaskStatus, TaskType,
};
pub use orbit_types::{
    redact_sensitive_env_error, redact_sensitive_env_json, redact_sensitive_env_option,
    redact_sensitive_env_text,
};
pub use runtime::OrbitRuntime;
