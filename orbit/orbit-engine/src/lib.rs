//! Activity and job execution engine with template rendering and retry logic.
//!
//! Orchestrates the full lifecycle of running an [`Activity`] or [`Job`]: resolving
//! input via Handlebars templates, delegating to the appropriate [`ActivityExecutor`],
//! recording step results, and handling retries and stale-run recovery.
//!
//! # Role
//! Depends on `orbit-agent`, `orbit-exec`, `orbit-store`, `orbit-tools`, and
//! `orbit-types`. Consumed by `orbit-core`, which constructs an [`EngineHost`] and
//! uses the public runner functions to drive execution.
//!
//! # Key exports
//! - [`run_activity_direct`] — execute a single activity with given input
//! - [`run_job_with_input`] — run a full job pipeline, stepping through all `JobStep`s
//! - [`recover_stale_active_run_for_job`] — resume or cancel orphaned job runs
//! - [`EngineHost`] / [`RuntimeHost`] / [`JobRunHost`] / [`TaskHost`] — host traits
//!   that orbit-core implements to provide store access and event emission
//! - [`ExecutionContext`] — per-run context (working directory, actor, tool context)
//! - [`JobRunResult`] — outcome of a completed job run
//!
//! # Dependency direction
//! `orbit-types`, `orbit-agent`, `orbit-exec`, `orbit-store`, `orbit-tools`
//! → `orbit-engine` → orbit-core

mod activity_runner;
mod context;
mod executor;
mod job_runner;
mod template;

pub use activity_runner::{
    activity_skill_refs_from_spec_config, run_activity_direct, validate_activity_input_schema,
    validate_activity_output_schema,
};
pub use context::{
    ACTIVITY_EXECUTION_FAILED, AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED,
    AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, AgentProtocolHost, AttemptOutcome,
    DirectActivityRunOutcome, EngineHost, EnvironmentHost, ExecutionContext, JobRunHost,
    JobRunResult, RuntimeHost, STALE_RUN_GRACE_SECONDS, TaskAutomationUpdate, TaskHost,
    execution_working_directory, execution_working_directory_with_task, input_workspace_path,
    redact_attempt_outcome, step_output_for_following_input,
};
pub use job_runner::{
    recover_stale_active_run_for_job, retry_job_run_from_step, run_job_with_input,
};
