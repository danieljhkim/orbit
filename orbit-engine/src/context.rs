use orbit_agent::AgentConfig;
use orbit_exec::EnvironmentMode;
use orbit_store::JobRunStepParams;
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, Job, JobRun, JobRunState, JobTargetType, OrbitError, OrbitEvent, Role, Task,
    TaskStatus, redact_sensitive_env_json, redact_sensitive_env_option,
};
use serde_json::Value;

pub const AGENT_PROTOCOL_VIOLATION: &str = "AGENT_PROTOCOL_VIOLATION";
pub const AGENT_INVOCATION_FAILED: &str = "AGENT_INVOCATION_FAILED";
pub const AGENT_COMMIT_FAILED: &str = "AGENT_COMMIT_FAILED";
pub const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
pub const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
pub const STALE_RUN_GRACE_SECONDS: u64 = 30;

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub activity: Activity,
    pub job: Option<Job>,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub env_extra: Vec<String>,
    pub input: Value,
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
    pub branch: Option<String>,
    pub pr_number: Option<String>,
}

pub trait EngineHost {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError>;
    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError>;
    fn get_pending_or_running_job_run(&self, job_id: &str) -> Result<Option<JobRun>, OrbitError>;
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

    fn agent_config_for(&self, agent_cli: &str) -> Result<AgentConfig, OrbitError>;
    fn execution_environment_mode(&self, env_extra: &[String]) -> EnvironmentMode;
    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String>;
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

    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError>;
    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError>;
    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError>;
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
