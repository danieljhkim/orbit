pub mod activity_runner;
pub mod job_runner;

use orbit_types::{
    Activity, Job, JobRunState, redact_sensitive_env_json, redact_sensitive_env_option,
};
use serde_json::Value;

pub(crate) const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
pub(crate) const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
pub(crate) const AGENT_COMMIT_FAILED: &str = "AGENT_COMMIT_FAILED";
pub(crate) const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
pub(crate) const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
pub(crate) const STALE_RUN_GRACE_SECONDS: u64 = 30;

#[derive(Debug, Clone)]
pub(crate) struct ExecutionContext {
    pub(crate) activity: Activity,
    pub(crate) job: Option<Job>,
    pub(crate) agent_cli: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) env_extra: Vec<String>,
    pub(crate) input: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct AttemptOutcome {
    pub(crate) state: JobRunState,
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) response_json: Option<Value>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) protocol_violation: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DirectActivityRunOutcome {
    pub(crate) state: JobRunState,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) protocol_violation: bool,
}

pub(crate) fn step_output_for_following_input<'a>(
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

pub(crate) fn input_workspace_path(input: &Value) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get("workspace_path"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn execution_working_directory(execution: &ExecutionContext) -> Option<String> {
    execution
        .activity
        .workspace_path
        .clone()
        .or_else(|| input_workspace_path(&execution.input))
}

pub(crate) fn redact_attempt_outcome(mut outcome: AttemptOutcome) -> AttemptOutcome {
    outcome.response_json = outcome.response_json.map(redact_sensitive_env_json);
    outcome.error_message = redact_sensitive_env_option(outcome.error_message);
    outcome
}
