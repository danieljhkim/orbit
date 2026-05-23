//! v2 activity runtime. Phase 2 coexistence layer — the v1 runtime is untouched.
//!
//! Depends on `orbit_common::types::activity_job` for the type surface (activity/spec/audit
//! shapes, tool-allowlist helpers). This module wires those types to the
//! engine's executor infrastructure and to the loop-engine audit pipeline.

pub mod agent_loop_driver;
pub mod agent_role;
pub mod audit_writer;
pub mod cli_runner;
pub mod dispatcher;
pub mod groundhog;
pub mod job_executor;
pub mod orbit_tool_executor;
pub mod sqlite_sink;
pub mod tool_enforcement;
pub mod workspace;

#[cfg(test)]
mod tests;

pub use agent_loop_driver::{
    drive_agent_loop, drive_agent_loop_with_session, drive_agent_loop_with_tool_context,
    reset_replay_transport,
};
pub use agent_role::{ResolvedAgentSettings, apply_resolved_settings, resolve_agent_settings};
pub use audit_writer::{V2AuditWriter, WriteError};
pub use cli_runner::run_cli_backend;
pub use dispatcher::{
    DispatchError, DispatchOutcome, ResolvedCliExecutor, ResolvedSandbox, V2DispatchInput,
    V2RuntimeHost, dispatch_v2_activity,
};
pub use job_executor::{
    JobOutcome, execute_job, resolve_job_catalog_refs_for_execution, validate_job,
};
pub use orbit_tool_executor::OrbitToolCallExecutor;
pub use sqlite_sink::V2SqliteSink;
pub use tool_enforcement::{EnforcedAuditSink, EnforcementDecision};
pub use workspace::resolve_subprocess_cwd;
