//! Execution-context primitives shared by the executors and the engine flows.
//!
//! Split by concern; every item is re-exported here so `crate::context::X`
//! paths (and the crate-root re-exports in `lib.rs`) stay stable:
//! - [`execution`] — the [`ExecutionContext`] passed through dispatch, plus
//!   working-directory resolution helpers.
//! - [`outcome`] — attempt/run outcome types, error-code constants, and
//!   workflow-failure helpers.
//! - [`hosts`] — the host trait boundary (`JobRunHost`, `TaskHost`,
//!   `EnvironmentHost`, `RuntimeHost`, ...) and the task-update param types.
//! - [`executor_host`] — [`ExecutorHost`] and the narrowed per-executor
//!   facades that delegate back to the full host.
//! - [`env`] — environment-variable resolution (`env_set` overrides and
//!   `ORBIT_*` state vars) applied on top of an `EnvironmentMode`.

mod env;
mod execution;
mod executor_host;
mod hosts;
mod outcome;

#[cfg(test)]
mod tests;

pub use env::{apply_env_set, inject_state_env, state_env_vars};
pub use execution::{
    ExecutionContext, execution_working_directory, execution_working_directory_with_task,
    input_workspace_path,
};
pub use executor_host::ExecutorHost;
pub use hosts::{
    AgentProtocolHost, AgentRoleConfig, EngineHost, EnvironmentHost, ExecutorLookupHost,
    JobRunHost, PrConfig, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskHost,
    TaskReadHost, TaskWriteHost, ensure_task_can_enter_workflow,
};
pub use outcome::{
    ACTIVITY_EXECUTION_FAILED, AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED,
    AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, ActivityInvocationResult, AttemptOutcome,
    DirectActivityRunOutcome, JobRunResult, STALE_RUN_GRACE_SECONDS, WORKFLOW_RUN_FAILED_EVENT,
    blocked_workflow_failure_update, redact_attempt_outcome, workflow_failure_note,
};
