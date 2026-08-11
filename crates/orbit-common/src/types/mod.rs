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
//! `orbit-common::types` ← orbit-policy, orbit-exec, orbit-tools,
//!                         orbit-store, orbit-agent, orbit-engine,
//!                         orbit-core, orbit-cli

pub mod activity_job;
pub mod actor;
pub mod adr;
pub mod agent_family;
pub mod agent_pair;
pub mod artifact_ids;
pub mod audit;
pub mod audit_event;
pub mod auto_task;
pub mod error;
pub mod event;
pub mod execution_profile;
pub mod executor_def;
pub mod friction;
pub mod host;
pub mod id;
pub mod invocation;
pub mod job;
pub mod learning;
pub mod metrics;
pub mod policy_decision;
pub mod policy_def;
pub mod pricing;
pub mod registry_snapshot;
pub mod resource;
pub mod role;
pub mod routine;
pub mod run_state;
pub mod skill;
pub mod spoke_registration;
pub mod task;
pub mod task_artifacts;
pub mod task_plan;
pub mod tool;
pub mod tool_input;
pub mod workspace;

pub use activity_job::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, ActivityAsset, ActivityV2, ActivityV2Spec, AgentLoopSpec,
    AssetLoadError, BackoffStrategy, BranchOutcome, CoreDeterministicAction, DeterministicAction,
    DeterministicSpec, EngineDeterministicAction, FanInSpec, FanOutBlock, JobActivityRoles,
    JobAsset, JobKind, JobV2, JobV2Step, JobV2StepBody, JoinMode, LoopBlock, OnDenial,
    ParallelBlock, PipelineRef, RetrySpec, SchemaHeader, TargetStep, ToolAllowlistError,
    V2_TOOL_WILDCARD_ROOTS, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind, load_activity_asset,
    load_job_asset, tool_allowed, validate_tool_allowlist,
};
pub use actor::{
    ActorIdentity, agent_from_model, normalize_attribution_label,
    normalize_optional_attribution_label, provider_from_model,
};
pub use adr::{
    Adr, AdrStatus, LegacyValidation, legacy_id_for, normalize_adr_paths, normalize_adr_tags,
    validate_adr_id,
};
pub use agent_family::AgentFamily;
pub use agent_pair::{
    AgentModelPair, Crew, CrewAssignment, agent_family_from_cli, all_agent_families,
    infer_agent_family_from_model, normalize_agent_family_for_model, resolve_crew,
};
pub use artifact_ids::{
    is_valid_adr_id, is_valid_friction_id, is_valid_learning_id, validate_friction_id,
};
pub use audit::Audit;
pub use audit_event::{AuditEvent, AuditEventStatus, AuditStats, audit_execution_id};
pub use auto_task::{
    AUTO_TASK_SCHEMA_VERSION, AUTO_TASK_TAG_PREFIX, AutoTaskDefinition, AutoTaskSchedule,
    AutoTaskTemplate, DedupePolicy, auto_task_tag, is_valid_auto_task_name, parse_auto_task_yaml,
};
pub use error::{
    ArtifactOrigin, ArtifactOriginMode, DependencyNotDelivered, NotFoundKind, OrbitError,
    WorkspaceClaimHeld,
};
pub use event::OrbitEvent;
pub use execution_profile::{
    CREW_DISCOVERY_SCHEMA_VERSION, CrewDiscoveryV1, EXECUTION_CONFIG_DIGEST_DOMAIN,
    EXECUTION_PROFILE_SCHEMA_VERSION, ExecutionProfileCrewV1, ExecutionProfileShipV1,
    ExecutionProfileV1, HostWorkspacePresence, ProjectionFreshness, SanitizedExecutionProfile,
    SanitizedWorkspacePresence, StoredExecutionProfile, ValidatedCrewProfile, WorkspaceOwnership,
    WorkspacePresenceDeclaration,
};
pub use executor_def::{
    ExecutorDef, ExecutorSandboxKind, ExecutorType, ModelPairOverride, StdoutFormat,
};
pub use friction::{FrictionEntry, FrictionFrontmatter, FrictionRecord, FrictionStatus};
pub use host::{
    HostAlias, HostNameResolution, HostRecord, HostRegistration, HostStatus,
    REGISTRY_IDENTIFIER_MAX_BYTES, validate_host_id, validate_machine_id,
    validate_registry_identifier,
};
pub use id::OrbitId;
pub use invocation::{InvocationTrace, TokenUsage, ToolCallTrace};
pub use job::{
    AgentCommitRequest, AgentResponseEnvelope, AgentRunError, Job, JobRun, JobRunState, JobRunStep,
    JobScheduleState, JobStep, JobTargetType, KnowledgeRunMetrics, RunEvent, StepCondition,
    default_job_max_active_runs, default_max_iterations, default_retry_backoff_seconds,
};
pub use learning::{
    DEFAULT_LEARNING_REMINDER_PER_CALL_CAP, DEFAULT_LEARNING_REMINDER_SESSION_CAP, EvidenceKind,
    Learning, LearningEvidence, LearningInjectionCaps, LearningInjectionState, LearningReminder,
    LearningScope, LearningStatus, normalize_learning_paths, normalize_learning_tags,
    prepend_reminder_block, render_reminder_block,
};
pub use metrics::MetricsEntry;
pub use policy_decision::PolicyDecision;
pub use policy_def::{
    DEFAULT_POLICY_NAME, FsCheckResult, FsOperation, FsProfile, PolicyDef, ResolvedFsProfile,
    UNRESTRICTED_FS_PROFILE,
};
pub use pricing::{InputTokenBasis, PriceRow, derive_cost_usd, normalize_token_usage};
pub use registry_snapshot::{
    REGISTRY_CACHE_SCHEMA_VERSION, REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistryAliasV1,
    RegistryCacheV1, RegistryHostV1, RegistryPresenceV1, RegistryProfileV1, RegistrySnapshotV1,
    RegistryWorkspaceV1,
};
pub use resource::{
    EXECUTOR_RESOURCE_SCHEMA_VERSION, ExecutorResource, ExecutorResourceSpec,
    POLICY_RESOURCE_SCHEMA_VERSION, PolicyResource, PolicyResourceSpec, ResourceEnvelope,
    ResourceHeader, ResourceKind, ResourceMetadata, parse_policy_resource, validate_resource_name,
};
pub use role::Role;
pub use routine::{
    MissedRunPolicy, OverlapPolicy, ROUTINE_SCHEMA_VERSION, RoutineDefinition, RoutinePolicy,
    RoutineRetries, RoutineTarget, RoutineTrigger, parse_local_routine_yaml, parse_routine_yaml,
};
pub use run_state::PipelineState;
pub use skill::Skill;
pub use spoke_registration::{
    SPOKE_REGISTRATION_METHOD_V1, SPOKE_REGISTRATION_SCHEMA_VERSION,
    SpokeExecutionProfilePublicationV1, SpokeRegistrationFailureV1, SpokeRegistrationRequestV1,
    SpokeRegistrationResultV1, SpokeRegistrationStageV1,
};
pub use task::{
    DEFAULT_TASK_LIST_LIMIT, DependencyDeadEnd, ExternalRef, GITHUB_PR_EXTERNAL_REF_SYSTEM,
    NO_DIFF_EXPECTED_TAG, ResolvedTaskDependency, Task, TaskArtifact, TaskComment, TaskComplexity,
    TaskCreateStatus, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
    UnsatisfiableTaskDependency, build_task_status_index, media_type_for_artifact_path,
    normalize_task_dependencies, normalize_task_tags, prune_missing_context_files,
    push_external_ref_if_missing, resolve_task_dependencies, task_dependencies_ready,
    task_matches_tags, unmet_task_dependencies, unsatisfiable_task_dependencies,
    validate_task_dependencies,
};
pub use task_artifacts::{
    ArtifactManifestFileV2, ArtifactManifestV2, ORB_TASK_ID_MAX, ORB_TASK_ID_PREFIX,
    ORB_TASK_ID_WIDTH, TASK_ACCEPTANCE_FILE_NAME, TASK_ARTIFACT_FILES_DIR_NAME,
    TASK_ARTIFACT_MANIFEST_FILE_NAME, TASK_ARTIFACT_SCHEMA_VERSION, TASK_ARTIFACTS_DIR_NAME,
    TASK_COMMENTS_FILE_NAME, TASK_DESCRIPTION_FILE_NAME, TASK_ENVELOPE_FILE_NAME,
    TASK_EVENTS_FILE_NAME, TASK_EXECUTION_SUMMARY_FILE_NAME, TASK_PLAN_FILE_NAME, TaskCommentRowV2,
    TaskEnvelopeV2, TaskEventRowV2, TaskRelation, TaskRelationEdge, TaskRelationType,
    format_orb_task_id, format_task_id, is_valid_orb_task_id, is_valid_task_id_prefix,
    parse_task_number, task_id_prefix, validate_orb_task_id, validate_relative_artifact_path,
    validate_task_relations_for_source,
};
pub use task_plan::{TaskPlan, TaskPlanCheckpoint, TaskPlanSuccessCriterion, parse_task_plan};
pub use tool::{
    ExecutionResult, McpCapability, McpCapabilityPlacementMatrix, McpLeasedRun, McpToolDefinition,
    McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope, McpTransport, StoredTool,
    ToolParam, ToolSchema, ToolSessionContext, mcp_advertised_tool_name,
    mcp_capability_placement_matrix, validate_mcp_tool_definitions,
};
pub use tool_input::{
    RETIRED_TASK_ADD_INPUT_FIELDS, optional_csv_or_string_list_alias, optional_raw_string,
    optional_string, optional_string_alias, optional_string_list_alias, optional_u32_alias,
    required_string, split_csv, strip_retired_task_add_input_fields,
};
pub use workspace::{
    WORKSPACE_REGISTRY_SCHEMA_VERSION, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole,
    WorkspacePaths, WorkspaceRegistry, WorkspaceStatus,
};

#[cfg(test)]
mod tests;
