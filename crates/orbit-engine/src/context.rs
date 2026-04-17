use crate::executor::registry::ActivityExecutorRegistry;
use orbit_agent::AgentConfig;
use orbit_exec::EnvironmentMode;
use orbit_store::JobRunStepParams;
use orbit_store::{InvocationQuery, InvocationRecord};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, AgentModelPair, ExecutorDef, InvocationTrace, Job, JobRun, JobRunState,
    JobTargetType, KnowledgeRunMetrics, OrbitError, OrbitEvent, PipelineState, ReviewThread, Role,
    Task, TaskArtifact, TaskComment, TaskPriority, TaskStatus, redact_sensitive_env_json,
    redact_sensitive_env_option, resolve_agent_model_pair,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
pub const ACTIVITY_EXECUTION_FAILED: &str = "ACTIVITY_EXECUTION_FAILED";
pub const INPUT_VALIDATION_FAILED: &str = "INPUT_VALIDATION_FAILED";
pub const RUN_ABANDONED: &str = "RUN_ABANDONED";
pub const WORKFLOW_RUN_FAILED_EVENT: &str = "workflow_run_failed";
pub const STALE_RUN_GRACE_SECONDS: u64 = 30;

/// Returns `true` for error codes that indicate a transient infrastructure failure
/// where an automatic retry is safe. Returns `false` for deterministic failures
/// such as protocol violations or unclassified invocation errors.
pub fn is_transient_error(code: &str) -> bool {
    matches!(
        code,
        AGENT_TRANSPORT_FAILURE | AGENT_PROVIDER_OVERLOAD | AGENT_RATE_LIMIT | AGENT_TIMEOUT
    )
}

pub fn workflow_failure_note(
    job_id: &str,
    run_id: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> String {
    let error_code = error_code
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");
    let error_message = error_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");

    format!(
        "workflow run failed: job={job_id}, run_id={run_id}, error_code={error_code}, error={error_message}"
    )
}

pub fn blocked_workflow_failure_update(
    job_id: &str,
    run_id: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> TaskAutomationUpdate {
    TaskAutomationUpdate {
        status: Some(TaskStatus::Blocked),
        status_event: Some(WORKFLOW_RUN_FAILED_EVENT.to_string()),
        status_note: Some(workflow_failure_note(
            job_id,
            run_id,
            error_code,
            error_message,
        )),
        ..TaskAutomationUpdate::default()
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub activity: Activity,
    pub job: Option<Job>,
    pub agent_cli: String,
    pub model: Option<String>,
    pub model_tier: Option<String>,
    pub timeout_seconds: u64,
    pub env_extra: Vec<String>,
    /// Explicit env var key-value pairs that override same-named vars from
    /// `env_extra` or the global allowlist.
    pub env_set: std::collections::HashMap<String, String>,
    pub input: Value,
    /// When `true`, stream agent stderr to the terminal and tee stdout live.
    pub debug: bool,
    /// Accumulated outputs from completed steps, keyed by step id (or target_id).
    /// Used to populate the `steps` namespace in TemplateContext.
    pub steps_outputs: std::collections::HashMap<String, Value>,
    pub run_id: Option<String>,
    pub step_index: Option<u32>,
    pub state_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub state: JobRunState,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub invocation_trace: InvocationTrace,
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
            invocation_trace: InvocationTrace::default(),
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
            invocation_trace: InvocationTrace {
                duration_ms,
                ..InvocationTrace::default()
            },
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
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ActivityInvocationResult {
    pub response_json: Option<Value>,
    pub invocation_trace: InvocationTrace,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TaskAutomationUpdate {
    pub status: Option<TaskStatus>,
    pub plan: Option<String>,
    pub workspace_path: Option<Option<String>>,
    pub repo_root: Option<String>,
    pub pr_number: Option<String>,
    pub execution_summary: Option<String>,
    pub status_event: Option<String>,
    pub status_note: Option<String>,
    pub append_comments: Vec<TaskComment>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub review_threads: Option<Vec<ReviewThread>>,
    pub batch_id: Option<String>,
}

pub trait JobRunHost {
    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError>;
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError>;
    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError>;
    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
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
    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError>;
    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError>;
    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError>;
    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError>;
    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError>;
}

pub trait TaskReadHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError>;
    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError>;
    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError>;
}

pub trait TaskWriteHost {
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

pub trait TaskHost: TaskReadHost + TaskWriteHost {}

impl<T> TaskHost for T where T: TaskReadHost + TaskWriteHost + ?Sized {}

pub trait AgentProtocolHost {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError>;
    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError>;
}

pub trait EnvironmentHost {
    // ── Config accessors (implementors provide these) ──────────────────

    /// Returns provider-agnostic key-value configuration that is forwarded
    /// to `ProviderOptions::for_agent_cli`.  Each provider extracts the keys
    /// it cares about (e.g. Codex reads `"sandbox"` and `"approval_policy"`).
    fn agent_provider_config(&self) -> HashMap<String, String>;
    fn execution_env_inherit(&self) -> bool;
    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)>;
    fn orbit_root(&self) -> Option<String>;
    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)>;
    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String>;

    // ── Default implementations (use accessors above) ──────────────────

    fn agent_config_for(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Result<AgentConfig, OrbitError> {
        use orbit_agent::ProviderOptions;
        let config = self.agent_provider_config();
        let provider_options = ProviderOptions::for_agent_cli(agent_cli, &config)?;
        Ok(AgentConfig {
            command: agent_cli.to_string(),
            model: model.map(|m| m.to_string()),
            provider_options,
        })
    }

    fn execution_environment_mode(&self, env_extra: &[String]) -> EnvironmentMode {
        if self.execution_env_inherit() {
            EnvironmentMode::Inherit
        } else {
            let mut env = self.hydrated_env_allowlist(env_extra);
            if let Some(orbit_root) = self.orbit_root()
                && !env.iter().any(|(k, _)| k == "ORBIT_ROOT")
            {
                env.push(("ORBIT_ROOT".to_string(), orbit_root));
            }
            EnvironmentMode::ClearAndSet(env)
        }
    }

    fn validate_agent_cli(&self, cli: &str, model: Option<&str>) -> Result<(), OrbitError> {
        use orbit_agent::Agent;
        let cfg = AgentConfig::cli(cli)?.with_model(model);
        let _ = Agent::new(&cfg)?;
        Ok(())
    }
}

pub trait ExecutorLookupHost {
    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError>;
}

pub trait RuntimeHost {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError>;
    fn repo_root(&self) -> Result<String, OrbitError>;
    fn data_root(&self) -> &Path;
    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry;
    fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        input: Value,
        debug: bool,
    ) -> Result<JobRunResult, OrbitError>;
    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError>;
    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError>;
    fn invocation_records(
        &self,
        _query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        Ok(Vec::new())
    }
    fn invocation_records_for_job_run_and_activity(
        &self,
        job_run_id: &str,
        activity_id: &str,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        self.invocation_records(InvocationQuery {
            job_run_id: Some(job_run_id.to_string()),
            activity_id: Some(activity_id.to_string()),
            limit: 1_000_000,
            ..InvocationQuery::default()
        })
    }
    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError>;
    fn invoke_activity(
        &self,
        _activity: Activity,
        _agent_cli: &str,
        _model: Option<&str>,
        _input: Value,
        _timeout_seconds: u64,
        _debug: bool,
    ) -> Result<ActivityInvocationResult, OrbitError> {
        Err(OrbitError::Execution(
            "invoke_activity is not implemented for this host".to_string(),
        ))
    }
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
    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        resolve_agent_model_pair(agent_cli)
    }
    fn canonical_model_name(&self, _agent_cli: &str, model: Option<&str>) -> Option<String> {
        model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }
    fn scoring_enabled(&self) -> bool;
    fn graph_editing(&self) -> bool;
    fn scoreboard_dir(&self) -> &Path;
    fn persist_invocation_trace(
        &self,
        _job_run_id: &str,
        _execution: &ExecutionContext,
        _trace: &InvocationTrace,
    ) -> Result<(), OrbitError> {
        Ok(())
    }
}

/// Aggregates the store/runtime traits needed by the top-level engine flows
/// (job orchestration, reconciliation, stale recovery). Executor dispatch uses
/// [`ExecutorHost`] instead of taking this full boundary directly.
pub trait EngineHost:
    JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost + Sync
{
}

impl<T> EngineHost for T where
    T: JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost + Sync
{
}

#[derive(Clone, Copy)]
pub struct ExecutorHost<'a> {
    runtime: &'a (dyn RuntimeHost + Sync),
    task_reader: &'a (dyn TaskReadHost + Sync),
    task_writer: &'a (dyn TaskWriteHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
    agent_protocol: &'a (dyn AgentProtocolHost + Sync),
    executor_lookup: &'a (dyn ExecutorLookupHost + Sync),
}

impl<'a> ExecutorHost<'a> {
    pub fn new<H>(host: &'a H) -> Self
    where
        H: RuntimeHost + TaskHost + EnvironmentHost + AgentProtocolHost + ExecutorLookupHost + Sync,
    {
        Self {
            runtime: host,
            task_reader: host,
            task_writer: host,
            environment: host,
            agent_protocol: host,
            executor_lookup: host,
        }
    }

    pub fn agent(self) -> AgentExecutorHost<'a> {
        AgentExecutorHost {
            task_reader: self.task_reader,
            environment: self.environment,
            agent_protocol: self.agent_protocol,
            executor_lookup: self.executor_lookup,
        }
    }

    pub fn cli(self) -> CliCommandExecutorHost<'a> {
        CliCommandExecutorHost {
            task_reader: self.task_reader,
            environment: self.environment,
        }
    }

    pub fn automation(self) -> AutomationExecutorHost<'a> {
        AutomationExecutorHost {
            runtime: self.runtime,
            task_reader: self.task_reader,
            task_writer: self.task_writer,
            environment: self.environment,
        }
    }
}

#[derive(Clone, Copy)]
pub struct AgentExecutorHost<'a> {
    task_reader: &'a (dyn TaskReadHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
    agent_protocol: &'a (dyn AgentProtocolHost + Sync),
    executor_lookup: &'a (dyn ExecutorLookupHost + Sync),
}

impl TaskReadHost for AgentExecutorHost<'_> {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.task_reader.get_task(task_id)
    }

    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.task_reader.get_task_artifacts(task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.task_reader
            .list_tasks_filtered(status, priority, parent_id, batch_id)
    }
}

impl EnvironmentHost for AgentExecutorHost<'_> {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        self.environment.agent_provider_config()
    }

    fn execution_env_inherit(&self) -> bool {
        self.environment.execution_env_inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.hydrated_env_allowlist(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        self.environment.orbit_root()
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.cli_command_environment(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.environment
            .missing_required_environment_vars(required_env_vars)
    }
}

impl AgentProtocolHost for AgentExecutorHost<'_> {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        self.agent_protocol
            .build_agent_stdin_envelope_payload(execution)
    }

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
        self.agent_protocol
            .execute_commit_request_if_present(result)
    }
}

impl ExecutorLookupHost for AgentExecutorHost<'_> {
    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.executor_lookup.get_executor_def(name)
    }
}

#[derive(Clone, Copy)]
pub struct CliCommandExecutorHost<'a> {
    task_reader: &'a (dyn TaskReadHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
}

impl TaskReadHost for CliCommandExecutorHost<'_> {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.task_reader.get_task(task_id)
    }

    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.task_reader.get_task_artifacts(task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.task_reader
            .list_tasks_filtered(status, priority, parent_id, batch_id)
    }
}

impl EnvironmentHost for CliCommandExecutorHost<'_> {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        self.environment.agent_provider_config()
    }

    fn execution_env_inherit(&self) -> bool {
        self.environment.execution_env_inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.hydrated_env_allowlist(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        self.environment.orbit_root()
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.cli_command_environment(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.environment
            .missing_required_environment_vars(required_env_vars)
    }
}

#[derive(Clone, Copy)]
pub struct AutomationExecutorHost<'a> {
    runtime: &'a (dyn RuntimeHost + Sync),
    task_reader: &'a (dyn TaskReadHost + Sync),
    task_writer: &'a (dyn TaskWriteHost + Sync),
    environment: &'a (dyn EnvironmentHost + Sync),
}

impl TaskReadHost for AutomationExecutorHost<'_> {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.task_reader.get_task(task_id)
    }

    fn get_task_artifacts(&self, task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.task_reader.get_task_artifacts(task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.task_reader
            .list_tasks_filtered(status, priority, parent_id, batch_id)
    }
}

impl TaskWriteHost for AutomationExecutorHost<'_> {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.task_writer.start_task(task_id, note, comment)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.task_writer.update_task_from_activity(
            task_id,
            status,
            execution_summary,
            comment,
            note,
        )
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        self.task_writer
            .apply_task_automation_update(task_id, update)
    }
}

impl EnvironmentHost for AutomationExecutorHost<'_> {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        self.environment.agent_provider_config()
    }

    fn execution_env_inherit(&self) -> bool {
        self.environment.execution_env_inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.hydrated_env_allowlist(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        self.environment.orbit_root()
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.environment.cli_command_environment(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.environment
            .missing_required_environment_vars(required_env_vars)
    }
}

impl RuntimeHost for AutomationExecutorHost<'_> {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        self.runtime.record_event(event)
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        self.runtime.repo_root()
    }

    fn data_root(&self) -> &Path {
        self.runtime.data_root()
    }

    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        self.runtime.activity_executor_registry()
    }

    fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        input: Value,
        debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        self.runtime
            .run_job_now_with_input_debug(job_id, input, debug)
    }

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        self.runtime
            .validate_activity_target_exists(target_type, target_id)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.runtime.get_job(job_id)
    }

    fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        self.runtime.invocation_records(query)
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        self.runtime
            .run_tool_with_context_and_role(name, input, role, tool_context)
    }

    fn invoke_activity(
        &self,
        activity: Activity,
        agent_cli: &str,
        model: Option<&str>,
        input: Value,
        timeout_seconds: u64,
        debug: bool,
    ) -> Result<ActivityInvocationResult, OrbitError> {
        self.runtime
            .invoke_activity(activity, agent_cli, model, input, timeout_seconds, debug)
    }

    fn maybe_create_failure_task(
        &self,
        job_id: &str,
        run_id: &str,
        error_code: &str,
        error_message: &str,
        agent: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), OrbitError> {
        self.runtime.maybe_create_failure_task(
            job_id,
            run_id,
            error_code,
            error_message,
            agent,
            model,
        )
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.runtime.resolved_agent_model_pair(agent_cli)
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        self.runtime.canonical_model_name(agent_cli, model)
    }

    fn scoring_enabled(&self) -> bool {
        self.runtime.scoring_enabled()
    }

    fn graph_editing(&self) -> bool {
        self.runtime.graph_editing()
    }

    fn scoreboard_dir(&self) -> &Path {
        self.runtime.scoreboard_dir()
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        execution: &ExecutionContext,
        trace: &InvocationTrace,
    ) -> Result<(), OrbitError> {
        self.runtime
            .persist_invocation_trace(job_run_id, execution, trace)
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

/// Resolve the working directory for an execution context, falling back to the
/// task's workspace_path when neither the activity nor input provides one.
/// This is the preferred variant for agent_invoke and cli_command executors
/// where a [`TaskHost`] is available.
pub fn execution_working_directory_with_task<H: TaskReadHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    execution_working_directory(execution).or_else(|| {
        execution
            .input
            .get("task_id")
            .and_then(Value::as_str)
            .and_then(|task_id| host.get_task(task_id).ok())
            .and_then(|task| task.workspace_path)
    })
}

/// Resolve `${VAR}` references in a value string from the parent environment.
/// Returns an empty string and logs a warning when the referenced var is not set.
/// Previously the literal `${VAR}` was passed through, which caused tools like `gh`
/// to receive an invalid token value.
fn resolve_env_refs(value: &str) -> String {
    if let Some(inner) = value.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        match std::env::var(inner) {
            Ok(resolved) => resolved,
            Err(_) => {
                eprintln!(
                    "orbit: warning: env_set references ${{{inner}}} but it is not set in the environment"
                );
                String::new()
            }
        }
    } else {
        value.to_string()
    }
}

/// Apply explicit key-value env vars (`env_set`) on top of an already-resolved
/// [`EnvironmentMode`].  Values may contain `${VAR}` references that are
/// resolved from the parent environment.  Entries in `env_set` override
/// same-named vars.
pub fn apply_env_set(
    mode: EnvironmentMode,
    env_set: &std::collections::HashMap<String, String>,
) -> EnvironmentMode {
    if env_set.is_empty() {
        return mode;
    }
    let apply = |pairs: &mut Vec<(String, String)>| {
        for (key, raw_value) in env_set {
            let value = resolve_env_refs(raw_value);
            if let Some(existing) = pairs.iter_mut().find(|(k, _)| k == key) {
                existing.1 = value;
            } else {
                pairs.push((key.clone(), value));
            }
        }
    };
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

pub fn state_env_vars(execution: &ExecutionContext) -> Vec<(String, String)> {
    let Some(run_id) = execution.run_id.as_ref() else {
        return Vec::new();
    };
    let Some(step_index) = execution.step_index else {
        return Vec::new();
    };
    let Some(state_dir) = execution.state_dir.as_ref() else {
        return Vec::new();
    };
    vec![
        ("ORBIT_RUN_ID".to_string(), run_id.clone()),
        ("ORBIT_STEP_INDEX".to_string(), step_index.to_string()),
        (
            "ORBIT_STATE_DIR".to_string(),
            state_dir.to_string_lossy().into_owned(),
        ),
    ]
}

pub fn inject_state_env(mode: EnvironmentMode, execution: &ExecutionContext) -> EnvironmentMode {
    let state_env = state_env_vars(execution);
    if state_env.is_empty() {
        return mode;
    }
    let apply = |pairs: &mut Vec<(String, String)>| {
        for (key, value) in &state_env {
            if let Some(existing) = pairs
                .iter_mut()
                .find(|(existing_key, _)| existing_key == key)
            {
                existing.1 = value.clone();
            } else {
                pairs.push((key.clone(), value.clone()));
            }
        }
    };
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

pub fn redact_attempt_outcome(mut outcome: AttemptOutcome) -> AttemptOutcome {
    outcome.response_json = outcome.response_json.map(redact_sensitive_env_json);
    outcome.error_message = redact_sensitive_env_option(outcome.error_message);
    outcome
}
