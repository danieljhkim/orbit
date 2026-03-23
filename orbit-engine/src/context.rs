use orbit_agent::AgentConfig;
use orbit_exec::EnvironmentMode;
use orbit_store::JobRunStepParams;
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, Job, JobRun, JobRunState, JobTargetType, OrbitError, OrbitEvent, Role, Task,
    TaskStatus, redact_sensitive_env_json, redact_sensitive_env_option,
};
use serde_json::Value;
use std::path::Path;

pub const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
pub const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
pub const AGENT_COMMIT_FAILED: &str = "AGENT_COMMIT_FAILED";
pub const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
/// WebSocket/HTTPS connection failure — safe to retry.
pub const AGENT_TRANSPORT_FAILURE: &str = "AGENT_TRANSPORT_FAILURE";
/// HTTP 5xx from provider (overloaded/unavailable) — safe to retry.
pub const AGENT_PROVIDER_OVERLOAD: &str = "AGENT_PROVIDER_OVERLOAD";
/// HTTP 429 rate-limit from provider — safe to retry with backoff.
pub const AGENT_RATE_LIMIT: &str = "AGENT_RATE_LIMIT";
/// Agent exited 0 but produced no parseable JSON envelope — safe to retry.
pub const AGENT_OUTPUT_MISSING: &str = "AGENT_OUTPUT_MISSING";
pub const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
pub const RUN_ABANDONED: &str = "RUN_ABANDONED";
pub const STALE_RUN_GRACE_SECONDS: u64 = 30;

/// Returns `true` for error codes that indicate a transient infrastructure failure
/// where an automatic retry is safe. Returns `false` for deterministic failures
/// such as protocol violations or unclassified invocation errors.
pub fn is_transient_error(code: &str) -> bool {
    matches!(
        code,
        AGENT_TRANSPORT_FAILURE
            | AGENT_PROVIDER_OVERLOAD
            | AGENT_RATE_LIMIT
            | AGENT_TIMEOUT
            | AGENT_OUTPUT_MISSING
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_codes_are_retryable() {
        assert!(is_transient_error(AGENT_TRANSPORT_FAILURE));
        assert!(is_transient_error(AGENT_PROVIDER_OVERLOAD));
        assert!(is_transient_error(AGENT_RATE_LIMIT));
        assert!(is_transient_error(AGENT_TIMEOUT));
    }

    #[test]
    fn non_transient_codes_are_not_retryable() {
        assert!(!is_transient_error(AGENT_PROTOCOL_VIOLATION));
        assert!(!is_transient_error(AGENT_INVOCATION_FAILED));
        assert!(!is_transient_error(AGENT_COMMIT_FAILED));
        assert!(!is_transient_error(ACTIVITY_EXECUTION_FAILED));
        assert!(!is_transient_error("UNKNOWN_CODE"));
    }

    #[test]
    fn agent_output_missing_is_retryable() {
        assert!(is_transient_error(AGENT_OUTPUT_MISSING));
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub activity: Activity,
    pub job: Option<Job>,
    pub agent_cli: String,
    pub model: Option<String>,
    pub timeout_seconds: u64,
    pub env_extra: Vec<String>,
    pub input: Value,
    /// When `true`, stream agent stderr to the terminal and tee stdout live.
    pub debug: bool,
}

#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub state: JobRunState,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub response_json: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub protocol_violation: bool,
    /// Number of retries that occurred before this final outcome (0 = first attempt succeeded/failed).
    pub retry_count: u32,
}

impl AttemptOutcome {
    pub fn failed(error_code: &str, message: String) -> Self {
        Self {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(error_code.to_string()),
            error_message: Some(message),
            protocol_violation: false,
            retry_count: 0,
        }
    }

    pub fn success(exit_code: i32, duration_ms: u64, response_json: Value) -> Self {
        Self {
            state: JobRunState::Success,
            exit_code: Some(exit_code),
            duration_ms: Some(duration_ms),
            response_json: Some(response_json),
            error_code: None,
            error_message: None,
            protocol_violation: false,
            retry_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirectActivityRunOutcome {
    pub state: JobRunState,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub protocol_violation: bool,
}

#[derive(Debug, Clone)]
pub struct JobRunResult {
    pub job_id: String,
    pub run_id: String,
    pub state: JobRunState,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TaskAutomationUpdate {
    pub status: Option<TaskStatus>,
    pub workspace_path: Option<String>,
    pub repo_root: Option<String>,
    pub pr_number: Option<String>,
    pub execution_summary: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
}

pub trait JobRunHost {
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<JobRun, OrbitError>;
    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError>;
    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, OrbitError>;
    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError>;
    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError>;
    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError>;
}

pub trait TaskHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError>;
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError>;
    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError>;
    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError>;
}

pub trait AgentProtocolHost {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError>;
    fn validate_skill_output_schema(
        &self,
        activity: &Activity,
        envelope: &orbit_types::AgentResponseEnvelope,
    ) -> Result<(), OrbitError>;
    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError>;
}

pub trait EnvironmentHost {
    fn agent_config_for(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Result<AgentConfig, OrbitError>;
    fn execution_environment_mode(&self, env_extra: &[String]) -> EnvironmentMode;
    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)>;
    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String>;
}

pub trait RuntimeHost {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError>;
    fn repo_root(&self) -> Result<String, OrbitError>;
    fn data_root(&self) -> &Path;
    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError>;
    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError>;
    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError>;
    /// Create a task capturing a job run failure, skipping creation if an open
    /// task for the same `job_id` + `error_code` combination already exists.
    /// When `agent` and `model` are provided, they are recorded on the created
    /// task so attribution reflects the actual agent that was running.
    fn maybe_create_failure_task(
        &self,
        job_id: &str,
        run_id: &str,
        error_code: &str,
        error_message: &str,
        agent: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), OrbitError>;
    fn scoring_enabled(&self) -> bool;
}

/// Aggregates all five sub-traits required at the top-level engine boundary.
///
/// All five sub-traits are always needed together because:
/// - `ActivityExecutor::execute` takes `&dyn EngineHost` as a single dispatch target,
///   allowing each executor implementation to use whatever sub-traits it needs.
/// - `run_job_with_input` and `execute_single_attempt` call `executor.execute(host, ...)`,
///   which requires the full `EngineHost` bound on the host value passed in.
///
/// Individual free functions (e.g. `automation::execute`, `agent::execute`) use narrower
/// bounds where possible — `RuntimeHost + TaskHost`, `EnvironmentHost + AgentProtocolHost` —
/// but the trait object boundary at `ActivityExecutor::execute` forces `EngineHost` at the
/// top level.
pub trait EngineHost:
    JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost
{
}

impl<T> EngineHost for T where
    T: JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost
{
}

pub fn step_output_for_following_input<'a>(
    activity: &Activity,
    response_json: Option<&'a Value>,
) -> Option<&'a serde_json::Map<String, Value>> {
    match activity.spec_type.as_str() {
        "agent_invoke" => response_json
            .and_then(|value| value.get("result"))
            .and_then(Value::as_object),
        _ => response_json.and_then(Value::as_object),
    }
}

pub fn input_workspace_path(input: &Value) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get("workspace_path"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn execution_working_directory(execution: &ExecutionContext) -> Option<String> {
    execution
        .activity
        .workspace_path
        .clone()
        .or_else(|| input_workspace_path(&execution.input))
}

pub fn redact_attempt_outcome(mut outcome: AttemptOutcome) -> AttemptOutcome {
    outcome.response_json = outcome.response_json.map(redact_sensitive_env_json);
    outcome.error_message = redact_sensitive_env_option(outcome.error_message);
    outcome
}
