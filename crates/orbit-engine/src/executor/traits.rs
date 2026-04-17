use crate::context::{AttemptOutcome, ExecutionContext, ExecutorHost};

/// Pluggable executor for a single activity `spec_type`.
///
/// Each implementation handles one kind of activity (e.g. `"agent_invoke"`,
/// `"cli_command"`). Executors are registered into the runtime's
/// [`super::registry::ActivityExecutorRegistry`] during bootstrap and looked up by
/// [`ExecutionContext::activity.spec_type`] at run time.
///
/// ## Contract
/// - `spec_type` must return the exact string used in activity YAML files.
/// - `execute` is called once per attempt. Retry orchestration is handled by
///   the caller ([`crate::activity_runner::execute_with_retry`]); implementors
///   do **not** retry internally.
/// - Return [`AttemptOutcome::failed`] (never panic) on failure so the engine
///   can record the error and decide whether to retry.
pub trait ActivityExecutor: Send + Sync {
    /// The `spec_type` string this executor handles (e.g. `"agent_invoke"`).
    fn spec_type(&self) -> &str;

    /// Execute one attempt of the activity described by `execution`.
    ///
    /// The outcome is recorded by the engine regardless of success or failure.
    /// Sensitive values (env vars, tokens) are redacted after this returns —
    /// do not pre-redact them inside the implementation.
    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome;
}
