//! v2 activity/job runtime types. Phase 2 of the activity-job v2 migration.
//!
//! Coexists with the v1 `Activity`/`Job` types in this crate — neither is
//! altered by this module. A two-pass asset loader (`asset_loader`) peeks the
//! `schemaVersion` discriminator and dispatches to the appropriate typed
//! deserializer, producing either a `LegacyActivity`/`LegacyJob` (v1) or an
//! `ActivityV2`/`JobV2` (v2).

pub mod activity_v2;
pub mod asset_loader;
pub mod audit_envelope;
pub mod backend;
pub mod job_v2;
pub mod schema_header;
pub mod tool_allowlist;

pub use activity_v2::{
    ActivityV2, ActivityV2Spec, AgentLoopSpec, Backend, DeterministicSpec, OnDenial, Provider,
    ShellSpec,
};
pub use asset_loader::{
    ActivityAsset, ActivityV2Asset, AssetLoadError, JobAsset, JobV2Asset, load_activity_asset,
    load_job_asset,
};
pub use audit_envelope::{
    AUDIT_ENVELOPE_SCHEMA_VERSION, BranchOutcome, V2AuditEnvelope, V2AuditEvent, V2AuditEventKind,
};
pub use backend::{
    BackendConstraintError, HttpOnlyFeature, resolve_activity_backends, resolve_job_backends,
    validate_job_loop_session_backends,
};
pub use job_v2::{
    BackoffStrategy, FanInSpec, FanOutBlock, JobV2, JobV2Step, JobV2StepBody, JoinMode, LoopBlock,
    ParallelBlock, PipelineRef, RetrySpec, TargetStep,
};
pub use schema_header::SchemaHeader;
pub use tool_allowlist::{
    ToolAllowlistError, V2_TOOL_WILDCARD_ROOTS, tool_allowed, validate_tool_allowlist,
};

/// Type alias for the v1 `Activity` struct to clarify intent at call sites that
/// explicitly handle the legacy (v1) shape during v2 coexistence.
pub type LegacyActivity = crate::Activity;

/// Type alias for the v1 `Job` struct to clarify intent at call sites that
/// explicitly handle the legacy (v1) shape during v2 coexistence.
pub type LegacyJob = crate::Job;
