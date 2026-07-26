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
//!   direct_agent)
//! - [`ActivityExecutorRegistry`] — registry of executors (still wired,
//!   though v2 does not consult it at dispatch time)
//!
//! # Dependency direction
//! orbit-common, orbit-agent, orbit-exec, orbit-store, orbit-tools
//! → `orbit-engine` → orbit-core

mod activity_job;
mod condition;
mod context;
mod executor;
mod template;

#[cfg(test)]
mod tests;

pub use activity_job::{
    DispatchError, DispatchOutcome, EnforcedAuditSink, JobOutcome, ResolvedCliExecutor,
    ResolvedSandbox, V2AgentDispatchOverride, V2AuditWriter, V2DispatchInput, V2RuntimeHost,
    V2SqliteSink, dispatch_error_to_orbit, dispatch_v2_activity, drive_agent_loop, execute_job,
    execute_job_with_resume, reset_replay_transport, resolve_job_catalog_refs_for_execution,
    validate_job, validate_job_deterministic_actions,
};
pub use context::{
    AGENT_INVOCATION_FAILED, AGENT_TIMEOUT, ActivityInvocationResult, AgentProtocolHost,
    AgentRoleConfig, EnvironmentHost, ExecutionContext, ExecutorHost, ExecutorLookupHost,
    JobRunHost, PrConfig, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskHost,
    TaskReadHost, TaskWriteHost, WORKFLOW_RUN_FAILED_EVENT, blocked_workflow_failure_update,
    ensure_task_can_enter_workflow,
};
pub use executor::automation::vcs::{WorktreeGcOptions, WorktreeGcResult, collect_worktrees};
pub use executor::automation::{
    StateExecutionContext, execute_action as execute_deterministic_action,
};
pub use executor::registry::ActivityExecutorRegistry;
