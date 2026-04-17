//! Shared domain types, error definitions, and ID generation for the Orbit workspace.
//!
//! This is the leaf crate in the dependency graph — it has no internal Orbit
//! dependencies and is imported by every other crate in the workspace.
//!
//! # Role
//! Acts as the single source of truth for all cross-crate data structures.
//! All other crates depend on this crate; it depends on none of them.
//!
//! # Key exports
//! - [`OrbitError`] — workspace-wide error enum; all crates use this exclusively
//! - [`OrbitId`] — deterministic, human-readable ID generation
//! - [`Activity`], [`Job`], [`JobRun`], [`Task`], [`Skill`] — core domain types
//! - [`ExecutionResult`] — process execution output shared between orbit-exec and callers
//! - [`AuditEvent`], [`OrbitEvent`] — event types for the audit trail and event bus
//! - [`Role`], [`PolicyDecision`] — RBAC primitives consumed by orbit-policy
//!
//! # Dependency direction
//! `orbit-types` ← orbit-policy, orbit-exec, orbit-tools, orbit-store,
//!                  orbit-agent, orbit-engine, orbit-core, orbit-cli

pub mod activity;
pub mod actor;
pub mod agent_pair;
pub mod audit;
pub mod audit_event;
pub mod duel;
pub mod error;
pub mod event;
pub mod executor_def;
pub mod friction;
pub mod id;
pub mod invocation;
pub mod job;
pub mod metrics;
pub mod policy_decision;
pub mod policy_def;
pub mod redaction;
pub mod resource;
pub mod role;
pub mod run_state;
pub mod skill;
pub mod task;
pub mod tool;
pub mod workspace;

pub use activity::Activity;
pub use actor::{ActorIdentity, normalize_attribution_label, normalize_optional_attribution_label};
pub use agent_pair::{
    AgentModelPair, agent_family_from_cli, all_agent_families, infer_agent_family_from_model,
    resolve_agent_model_pair,
};
pub use audit::Audit;
pub use audit_event::{AuditEvent, AuditEventStatus, AuditStats};
pub use duel::{
    Ambiguity, ArbiterVerdict, Cost, Decision, DuelRun, EfficiencyMetrics, ImplementerStats,
    Outcome, PerCommentVerdict, PlannerSlot, PlanningDuelRun, PlanningEfficiency, PlanningOutcome,
    PlanningRoleAssignment, PlanningRoles, ReviewerStats, RoleAssignment, Roles, Scores, Severity,
    TaskClass, TaskScope, ValidIssuesBySeverity, Verdict,
};
pub use error::OrbitError;
pub use event::OrbitEvent;
pub use executor_def::ExecutorDef;
pub use friction::FrictionEntry;
pub use id::OrbitId;
pub use invocation::{InvocationTrace, TokenUsage, ToolCallTrace};
pub use job::{
    AgentCommitRequest, AgentResponseEnvelope, AgentRunError, Job, JobRun, JobRunState, JobRunStep,
    JobScheduleState, JobStep, JobTargetType, KnowledgeRunMetrics, RunEvent, StepCondition,
    default_job_max_active_runs, default_max_iterations, default_retry_backoff_seconds,
};
pub use metrics::MetricsEntry;
pub use policy_decision::PolicyDecision;
pub use policy_def::{FilesystemPolicy, PolicyDef, ProcessPolicy, ToolPolicy};
pub use redaction::{
    is_sensitive_env_name, redact_sensitive_env_error, redact_sensitive_env_json,
    redact_sensitive_env_option, redact_sensitive_env_text,
};
pub use resource::{
    ActivityResource, ActivityResourceSpec, ExecutorResource, ExecutorResourceSpec, JobResource,
    JobResourceSpec, PolicyResource, PolicyResourceSpec, RESOURCE_SCHEMA_VERSION, ResourceEnvelope,
    ResourceHeader, ResourceKind, ResourceMetadata,
};
pub use role::Role;
pub use run_state::PipelineState;
pub use skill::Skill;
pub use task::{
    ReviewMessage, ReviewThread, ReviewThreadStatus, Task, TaskArtifact, TaskComment,
    TaskComplexity, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
    prune_missing_context_files,
};
pub use tool::{ExecutionResult, StoredTool, ToolParam, ToolSchema};
pub use workspace::{Workspace, WorkspacePaths, WorkspaceRegistry, WorkspaceStatus};
