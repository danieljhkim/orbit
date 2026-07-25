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
mod context;
mod executor;
mod job_runner;
mod template;

pub use activity_job::{
    DispatchError, DispatchOutcome, EnforcedAuditSink, EnforcementDecision, JobOutcome,
    OrbitToolCallExecutor, ResolvedAgentSettings, ResolvedCliExecutor, ResolvedSandbox,
    V2AuditWriter, V2DispatchInput, V2RuntimeHost, V2SqliteSink, WriteError,
    apply_resolved_settings, dispatch_error_to_orbit, dispatch_v2_activity, drive_agent_loop,
    drive_agent_loop_with_session, drive_agent_loop_with_tool_context, execute_job,
    execute_job_with_resume, reset_replay_transport, resolve_agent_settings,
    resolve_job_catalog_refs_for_execution, resolve_subprocess_cwd, run_cli_backend, validate_job,
};
pub use context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED,
    AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, ActivityInvocationResult, AgentProtocolHost,
    AgentRoleConfig, AttemptOutcome, DirectActivityRunOutcome, EngineHost, EnvironmentHost,
    ExecutionContext, ExecutorHost, ExecutorLookupHost, JobRunHost, JobRunResult, PrConfig,
    RuntimeHost, STALE_RUN_GRACE_SECONDS, TaskActivityUpdate, TaskAutomationUpdate, TaskHost,
    TaskReadHost, TaskWriteHost, WORKFLOW_RUN_FAILED_EVENT, blocked_workflow_failure_update,
    ensure_task_can_enter_workflow, execution_working_directory,
    execution_working_directory_with_task, input_workspace_path, redact_attempt_outcome,
    workflow_failure_note,
};
pub use executor::automation::vcs::{
    WorktreeGcOptions, WorktreeGcReport, WorktreeGcResult, collect_worktrees,
    resolve_shared_worktree_path, resolve_worktree_path_from_prefix,
};
pub use executor::automation::{
    StateExecutionContext, execute_action as execute_deterministic_action,
};
pub use executor::registry::ActivityExecutorRegistry;
