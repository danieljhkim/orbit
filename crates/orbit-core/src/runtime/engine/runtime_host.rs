use orbit_engine::{
    ActivityInvocationResult, ExecutionContext, RuntimeHost, execute_single_attempt,
    validate_activity_input_schema,
};
use orbit_store::{InvocationInsertParams, InvocationQuery, InvocationRecord, token_scoreboard};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, AgentModelPair, InvocationTrace, JobRunState, JobTargetType, OrbitError, OrbitEvent,
    Role, TaskPriority, TaskStatus, TaskType,
};
use serde_json::Value;

use super::identity::normalize_agent_name;
use super::invocation::{associated_task_ids, open_invocation_store};
use super::paths::current_repo_root;
use crate::OrbitRuntime;

impl RuntimeHost for OrbitRuntime {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        OrbitRuntime::record_event(self, event)
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        current_repo_root(self)
    }

    fn data_root(&self) -> &std::path::Path {
        self.context.data_root()
    }

    fn activity_executor_registry(&self) -> &orbit_engine::ActivityExecutorRegistry {
        self.activity_executor_registry()
    }

    fn run_job_now_with_input_debug(
        &self,
        job_id: &str,
        input: Value,
        debug: bool,
    ) -> Result<orbit_engine::JobRunResult, OrbitError> {
        OrbitRuntime::run_job_now_with_input_debug(self, job_id, input, debug)
    }

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        OrbitRuntime::validate_activity_target_exists(self, target_type, target_id)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
        self.stores().jobs().get(job_id)
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.configured_agent_model_pair(agent_cli)
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        self.canonical_model_for_agent(agent_cli, model)
    }

    fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        OrbitRuntime::invocation_records(self, query)
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        OrbitRuntime::run_tool_with_context_and_role(self, name, input, role, tool_context)
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
        if activity.spec_type != "agent_invoke" {
            return Err(OrbitError::InvalidInput(format!(
                "invoke_activity only supports agent_invoke activities, got '{}'",
                activity.spec_type
            )));
        }

        validate_activity_input_schema(&activity, &input)?;

        let execution = ExecutionContext {
            activity,
            job: None,
            agent_cli: agent_cli.to_string(),
            model: model.map(ToOwned::to_owned),
            model_tier: None,
            timeout_seconds,
            env_extra: vec![],
            env_set: std::collections::HashMap::new(),
            input,
            debug,
            steps_outputs: std::collections::HashMap::new(),
            run_id: None,
            step_index: None,
            state_dir: None,
        };
        let activity_id = execution.activity.id.clone();
        let outcome = execute_single_attempt(self, &execution);
        let duration_ms = outcome
            .duration_ms
            .unwrap_or(outcome.invocation_trace.duration_ms);

        if outcome.state != JobRunState::Success {
            let error_code = outcome
                .error_code
                .unwrap_or_else(|| outcome.state.to_string());
            let error_message = outcome.error_message.unwrap_or_else(|| {
                format!("activity '{activity_id}' finished in non-success state")
            });
            return if error_code == orbit_engine::AGENT_PROTOCOL_VIOLATION {
                Err(OrbitError::AgentProtocolViolation(error_message))
            } else {
                Err(OrbitError::Execution(format!(
                    "activity '{activity_id}' failed [{error_code}]: {error_message}"
                )))
            };
        }

        Ok(ActivityInvocationResult {
            response_json: outcome.response_json,
            invocation_trace: outcome.invocation_trace,
            exit_code: outcome.exit_code,
            duration_ms,
        })
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
        let title = format!("Job failure: {job_id} [{error_code}]");
        let tasks = self.stores().tasks().list()?;
        let already_open = tasks.iter().any(|t| {
            t.title == title
                && !matches!(
                    t.status,
                    TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
                )
        });
        if already_open {
            return Ok(());
        }
        let description = format!(
            "Job `{job_id}` failed during run `{run_id}` with error code `{error_code}`.\n\nError: {}",
            if error_message.is_empty() {
                "No error message provided."
            } else {
                error_message
            }
        );
        let _ = self.add_task_with_identity(
            crate::command::task::TaskAddParams {
                parent_id: None,
                title,
                description,
                acceptance_criteria: vec![
                    format!("Root cause for job `{job_id}` is identified"),
                    "A fix is implemented for the underlying issue".to_string(),
                    "The job completes successfully after verification".to_string(),
                ],
                plan: String::new(),
                comment: None,
                context_files: vec![],
                workspace_path: None,
                priority: TaskPriority::High,
                complexity: None,
                task_type: TaskType::Issue,
                system_created: true,
                source_task_id: None,
            },
            agent.map(ToOwned::to_owned),
            model.map(ToOwned::to_owned),
        );
        Ok(())
    }

    fn scoring_enabled(&self) -> bool {
        self.context.scoring_enabled()
    }

    fn graph_editing(&self) -> bool {
        self.context.graph_editing()
    }

    fn scoreboard_dir(&self) -> &std::path::Path {
        &self.context.paths().scoreboard_dir
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        execution: &ExecutionContext,
        trace: &InvocationTrace,
    ) -> Result<(), OrbitError> {
        let (agent, model) = self
            .canonical_agent_model_identity(Some(&execution.agent_cli), execution.model.as_deref());
        let store = open_invocation_store(self)?;
        store.insert_invocation_trace_record(&InvocationInsertParams {
            job_run_id: job_run_id.to_string(),
            activity_id: execution.activity.id.clone(),
            agent: agent.unwrap_or_else(|| normalize_agent_name(&execution.agent_cli)),
            model,
            task_ids: associated_task_ids(&execution.input),
            trace: trace.clone(),
        })?;

        if let Err(error) =
            token_scoreboard::write_token_scoreboard(&self.paths().scoreboard_dir, &store)
        {
            eprintln!("orbit: failed to refresh tokens scoreboard: {error}");
        }

        Ok(())
    }
}
