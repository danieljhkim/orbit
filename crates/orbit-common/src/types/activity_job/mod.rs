//! Activity/job runtime types and schemaVersion 2 asset loaders.

pub mod activity_v2;
pub mod asset_loader;
pub mod audit_envelope;
pub mod backend;
pub mod catalog;
pub mod job_v2;
pub mod schema_header;
pub mod tool_allowlist;

pub use activity_v2::{
    ActivityV2, ActivityV2Spec, AgentLoopSpec, AgentRole, Backend, DeterministicSpec,
    GroundhogSpec, OnDenial, Provider, ShellSpec,
};
pub use asset_loader::{
    ActivityAsset, AssetLoadError, JobAsset, load_activity_asset, load_job_asset,
};
pub use audit_envelope::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, BranchOutcome, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind,
};
pub use backend::{
    BackendConstraintError, HttpOnlyFeature, resolve_activity_backends, resolve_job_backends,
    validate_job_loop_session_backends,
};
pub use catalog::{
    ACTIVITY_REF_PREFIX, CatalogError, ResolveError, V2ActivityCatalog, resolve_job_target_refs,
};
pub use job_v2::{
    BackoffStrategy, FanInSpec, FanOutBlock, JobKind, JobV2, JobV2Step, JobV2StepBody, JoinMode,
    LoopBlock, ParallelBlock, PipelineRef, RetrySpec, TargetRef, TargetStep,
};
pub use schema_header::SchemaHeader;
pub use tool_allowlist::{
    ToolAllowlistError, V2_TOOL_WILDCARD_ROOTS, tool_allowed, validate_tool_allowlist,
};
