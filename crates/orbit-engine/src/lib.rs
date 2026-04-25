//! v2 activity/job execution engine with template rendering and retry logic.
//!
//! Orchestrates the full lifecycle of running a v2 activity or job:
//! resolving input via Handlebars templates, dispatching through
//! [`activity_job`], recording step results, and handling retries.
//!
//! # Role
//! Depends on `orbit-agent`, `orbit-exec`, `orbit-store`, `orbit-tools`, and
//! `orbit-common`. Consumed by `orbit-core`.
//!
//! # Key exports
//! - [`activity_job`] — v2 dispatcher, job executor, audit writer
//! - [`ExecutionContext`] / [`AttemptOutcome`] / [`ExecutorHost`] — host
//!   primitives still used by the kept v1 executors (cli_command,
//!   direct_agent, automation) and by the v2 `OrbitToolCallExecutor`
//! - [`ActivityExecutorRegistry`] — registry of executors (still wired,
//!   though v2 does not consult it at dispatch time)
//!
//! # Dependency direction
//! orbit-common, orbit-agent, orbit-exec, orbit-store, orbit-tools
//! → `orbit-engine` → orbit-core

pub mod activity_job;
mod checkpoint_verifier;
mod context;
mod executor;
mod job_runner;
mod template;
mod workspace_snapshot;

pub use checkpoint_verifier::{
    Criterion, CriterionOutcome, CriterionRun, DEFAULT_OUTPUT_CAP_BYTES, VerifierConfig,
    VerifierResult, verify_checkpoint, verify_checkpoint_with_config,
};
pub use context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED,
    AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, ActivityInvocationResult, AgentProtocolHost,
    AttemptOutcome, DirectActivityRunOutcome, EngineHost, EnvironmentHost, ExecutionContext,
    ExecutorHost, ExecutorLookupHost, JobRunHost, JobRunResult, RuntimeHost,
    STALE_RUN_GRACE_SECONDS, TaskAutomationUpdate, TaskHost, TaskReadHost, TaskWriteHost,
    execution_working_directory, execution_working_directory_with_task, input_workspace_path,
    redact_attempt_outcome,
};
pub use executor::automation::{
    StateExecutionContext, execute_action as execute_deterministic_action,
};
pub use executor::registry::ActivityExecutorRegistry;
pub use workspace_snapshot::{WorkspaceSnapshot, WorkspaceSnapshotRef};
