//! The host trait boundary between the engine and its runtime/store
//! implementors, plus the task-update param types those traits consume.

use crate::executor::registry::ActivityExecutorRegistry;
use orbit_agent::AgentConfig;
use orbit_common::types::InvocationTrace;
use orbit_common::types::activity_job::{AgentRole, Backend, Provider};
use orbit_common::types::{
    Activity, AgentModelPair, ExecutorDef, ExternalRef, Job, JobRun, JobRunState, JobTargetType,
    KnowledgeRunMetrics, OrbitError, OrbitEvent, PipelineState, ReviewThread, Role, Task,
    TaskArtifact, TaskComment, TaskHistoryEntry, TaskPriority, TaskStatus, all_agent_families,
};
use orbit_exec::EnvironmentMode;
use orbit_store::JobRunStepParams;
use orbit_store::{InvocationQuery, InvocationRecord};
use orbit_tools::ToolContext;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use super::execution::ExecutionContext;
use super::outcome::{ActivityInvocationResult, JobRunResult};

#[derive(Debug, Clone, Default)]
pub struct TaskAutomationUpdate {
    pub status: Option<TaskStatus>,
    pub plan: Option<String>,
    /// Default `None` = leave the task's `context_files` untouched. `Some(v)`
    /// replaces the field wholesale (mirrors `TaskDocumentUpdateParams.context_files`
    /// semantics in `orbit-store`). Only set deliberately — most automation
    /// call sites should leave this at `None`.
    pub context_files: Option<Vec<String>>,
    pub external_refs: Vec<ExternalRef>,
    pub execution_summary: Option<String>,
    pub status_event: Option<String>,
    pub status_note: Option<String>,
    pub append_comments: Vec<TaskComment>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub review_threads: Option<Vec<ReviewThread>>,
    pub job_run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TaskActivityUpdate {
    pub status: TaskStatus,
    pub execution_summary: Option<String>,
    pub comment: Option<String>,
    pub note: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
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
    fn get_task_comments(&self, _task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        Ok(Vec::new())
    }
    fn get_task_history(&self, _task_id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        Ok(Vec::new())
    }
    fn get_task_review_threads(&self, _task_id: &str) -> Result<Vec<ReviewThread>, OrbitError> {
        Ok(Vec::new())
    }
    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError>;
}

pub trait TaskWriteHost {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError>;
    fn admit_task_for_workflow(&self, task_id: &str, workflow: &str) -> Result<Task, OrbitError>;
    fn update_task_from_activity(
        &self,
        task_id: &str,
        update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError>;
    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError>;
}

pub trait TaskHost: TaskReadHost + TaskWriteHost {}

impl<T> TaskHost for T where T: TaskReadHost + TaskWriteHost + ?Sized {}

pub fn ensure_task_can_enter_workflow<H: TaskReadHost + ?Sized>(
    host: &H,
    task_id: &str,
    workflow: &str,
) -> Result<Task, OrbitError> {
    let task = host.get_task(task_id)?;
    if matches!(
        task.status,
        TaskStatus::Proposed
            | TaskStatus::Backlog
            | TaskStatus::Rejected
            | TaskStatus::Archived
            | TaskStatus::InProgress
    ) {
        return Ok(task);
    }

    Err(OrbitError::InvalidInput(format!(
        "task '{}' is in status '{}'; workflow admission for '{workflow}' requires 'proposed', 'backlog', 'rejected', 'archived', or 'in-progress'",
        task.id, task.status
    )))
}

pub trait AgentProtocolHost {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError>;
}

/// Resolved crew role assignment from `config.toml`. Each field
/// is independently optional — the resolver in
/// `crate::activity_job::agent_role` falls back to the inline activity value
/// for any field the config does not specify.
///
/// String fields from the on-disk `RawAgentRoleConfig` are parsed into the
/// strongly-typed activity-job enums at the orbit-core boundary; an
/// unrecognized provider/backend yields `None` for that field rather than
/// silently coercing dispatch to a wrong runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRoleConfig {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub backend: Option<Backend>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrConfig {
    pub task_url_template: Option<String>,
}

pub trait EnvironmentHost {
    // ── Config accessors (implementors provide these) ──────────────────

    /// Returns provider-agnostic key-value configuration that is forwarded
    /// to the selected provider factory so it can decode any provider-specific
    /// settings (for example Codex reads `"sandbox"` and `"approval_policy"`).
    fn agent_provider_config(&self) -> HashMap<String, String>;
    fn execution_env_inherit(&self) -> bool;
    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)>;
    fn orbit_root(&self) -> Option<String>;
    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)>;
    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String>;

    /// Resolved crew role assignment from the active workspace's
    /// `config.toml`, if any. The default returns `None`, which means
    /// dispatch falls back to the inline `provider`/`model`/`backend` on the
    /// activity. orbit-core's implementation reads the selected
    /// `[crews.<name>]` entry and parses the string fields into the
    /// strongly-typed activity-job enums.
    fn agent_role_config(&self, _role: AgentRole) -> Option<AgentRoleConfig> {
        None
    }

    // ── Default implementations (use accessors above) ──────────────────

    fn agent_config_for(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Result<AgentConfig, OrbitError> {
        let config = self.agent_provider_config();
        AgentConfig::from_cli_config(agent_cli, model, &config)
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
    fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        Err(OrbitError::Execution(format!(
            "cancel_job_run is not implemented for run '{run_id}'"
        )))
    }
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
    fn activity_implementer_identity(
        &self,
        _input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        Ok((None, None))
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
        let _ = agent_cli;
        None
    }
    fn duel_candidate_families(&self) -> Vec<String> {
        all_agent_families()
            .iter()
            .map(|family| (*family).to_string())
            .collect()
    }
    fn duel_orchestrator_model(&self, _family: &str) -> Option<String> {
        None
    }
    fn canonical_model_name(&self, _agent_cli: &str, model: Option<&str>) -> Option<String> {
        model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }
    fn scoring_enabled(&self) -> bool;
    fn graph_editing(&self) -> bool;
    /// Return the current agent model identity when this runtime is operating
    /// as an agent, or `None` when there is no model-bearing actor.
    fn actor_model_identity(&self) -> Option<String> {
        None
    }
    fn pr_config(&self) -> PrConfig {
        PrConfig::default()
    }
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
/// [`ExecutorHost`](super::executor_host::ExecutorHost) instead of taking this
/// full boundary directly.
pub trait EngineHost:
    JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost + Sync
{
}

impl<T> EngineHost for T where
    T: JobRunHost + TaskHost + AgentProtocolHost + EnvironmentHost + RuntimeHost + Sync
{
}
