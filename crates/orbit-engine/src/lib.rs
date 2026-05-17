#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy execution-engine surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! v2 activity/job execution engine with template rendering and retry logic.
//!
//! Orchestrates the full lifecycle of running a v2 activity or job:
//! resolving input via Handlebars templates, dispatching through the internal
//! activity runtime, recording step results, and handling retries.
//!
//! # Role
//! Depends on `orbit-agent`, `orbit-exec`, `orbit-store`, `orbit-tools`, and
//! `orbit-common`. Consumed by `orbit-core`.
//!
//! # Key exports
//! - v2 dispatcher, job executor, and audit writer types re-exported at the
//!   crate root
//! - [`ExecutionContext`] / [`AttemptOutcome`] / [`ExecutorHost`] — host
//!   primitives still used by the kept v1 executors (cli_command,
//!   direct_agent, automation) and by the v2 `OrbitToolCallExecutor`
//! - [`ActivityExecutorRegistry`] — registry of executors (still wired,
//!   though v2 does not consult it at dispatch time)
//!
//! # Dependency direction
//! orbit-common, orbit-agent, orbit-exec, orbit-store, orbit-tools
//! → `orbit-engine` → orbit-core

mod activity_job;
mod checkpoint_verifier;
mod context;
mod executor;
mod job_runner;
mod template;
mod workspace_snapshot;

pub use activity_job::{
    DispatchError, DispatchOutcome, EnforcedAuditSink, EnforcementDecision, JobOutcome,
    OrbitToolCallExecutor, ResolvedAgentSettings, ResolvedCliExecutor, ResolvedSandbox,
    V2AuditWriter, V2DispatchInput, V2JsonlSink, V2RuntimeHost, WriteError,
    apply_resolved_settings, dispatch_v2_activity, drive_agent_loop, drive_agent_loop_with_session,
    drive_agent_loop_with_tool_context, execute_job, reset_replay_transport,
    resolve_agent_settings, resolve_job_catalog_refs_for_execution, resolve_subprocess_cwd,
    run_cli_backend, validate_job,
};
pub use checkpoint_verifier::{
    Criterion, CriterionOutcome, CriterionRun, DEFAULT_OUTPUT_CAP_BYTES, VerifierConfig,
    VerifierResult, verify_checkpoint, verify_checkpoint_with_config,
};
pub use context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED,
    AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, ActivityInvocationResult, AgentProtocolHost,
    AgentRoleConfig, AttemptOutcome, DirectActivityRunOutcome, EngineHost, EnvironmentHost,
    ExecutionContext, ExecutorHost, ExecutorLookupHost, JobRunHost, JobRunResult, PrConfig,
    RuntimeHost, STALE_RUN_GRACE_SECONDS, TaskActivityUpdate, TaskAutomationUpdate, TaskHost,
    TaskReadHost, TaskWriteHost, execution_working_directory,
    execution_working_directory_with_task, input_workspace_path, redact_attempt_outcome,
};
pub use executor::automation::{
    StateExecutionContext, execute_action as execute_deterministic_action,
};
pub use executor::registry::ActivityExecutorRegistry;
pub use workspace_snapshot::{WorkspaceSnapshot, WorkspaceSnapshotRef};
