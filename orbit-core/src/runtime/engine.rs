use chrono::{DateTime, Utc};
use orbit_agent::{AgentConfig, ProviderOptions};
use orbit_engine::{
    AgentProtocolHost, EnvironmentHost, ExecutionContext, JobRunHost, RuntimeHost,
    TaskAutomationUpdate, TaskHost, activity_skill_refs_from_spec_config,
};
use orbit_exec::EnvironmentMode;
use orbit_store::{JobRunStepParams, TaskUpdateParams as StoreTaskUpdateParams};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, AgentCommitRequest, AgentResponseEnvelope, JobRun, JobRunState, JobTargetType,
    OrbitError, OrbitEvent, Role, Task, TaskPriority, TaskStatus, TaskType,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::paths;
use orbit_store::validate_instance_against_schema;

#[derive(Debug, Clone, Serialize)]
struct ExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    activity: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    job: Option<Value>,
    skills: Vec<ExecutionSkillEnvelope>,
    input: Value,
    memory: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ExecutionSkillEnvelope {
    id: String,
    content_hash: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

fn build_agent_stdin_envelope_payload(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
) -> Result<Vec<u8>, OrbitError> {
    let skill_refs = activity_skill_refs_from_spec_config(&execution.activity.spec_config)?;
    let skills = runtime.resolve_activity_skill_refs(&skill_refs)?;
    let envelope = ExecutionEnvelope {
        schema_version: 1,
        activity: activity_envelope_json(&execution.activity),
        job: execution.job.as_ref().map(|job| {
            json!({
                "id": job.job_id,
                "state": job.state,
                "default_input": job.default_input,
                "steps": job.steps.iter().map(|s| json!({
                    "target_type": s.target_type,
                    "target_id": s.target_id,
                    "agent_cli": s.agent_cli,
                    "model": s.model,
                    "timeout_seconds": s.timeout_seconds,
                })).collect::<Vec<_>>(),
            })
        }),
        skills: skills
            .into_iter()
            .map(|skill| ExecutionSkillEnvelope {
                id: skill.id,
                content_hash: skill.content_hash,
                content: skill.content,
                meta: skill.meta_raw,
            })
            .collect(),
        input: execution.input.clone(),
        memory: json!({}),
    };

    serde_json::to_vec(&envelope)
        .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
}

fn validate_skill_output_schema(
    runtime: &OrbitRuntime,
    activity: &Activity,
    envelope: &AgentResponseEnvelope,
) -> Result<(), OrbitError> {
    let skill_refs = activity_skill_refs_from_spec_config(&activity.spec_config)?;
    if skill_refs.is_empty() {
        // No skill_refs means no structured output contract; nothing to enforce.
        return Ok(());
    }
    let skills = runtime.resolve_activity_skill_refs(&skill_refs)?;
    // If no referenced skill defines an output schema, the skill_refs are context-only;
    // there is no structured output contract to enforce here.
    if skills.iter().all(|s| s.output_schema.is_none()) {
        return Ok(());
    }
    let Some(result) = envelope.result.as_ref() else {
        return Err(OrbitError::AgentProtocolViolation(
            "success response must include result payload".to_string(),
        ));
    };

    for skill in skills {
        let Some(schema) = skill.output_schema.as_ref() else {
            continue;
        };
        let context = format!("result does not match skill '{}' output schema", skill.id);
        if let Err(err) = validate_instance_against_schema(schema, result, &context) {
            return match err {
                OrbitError::SkillValidation(message) => {
                    Err(OrbitError::AgentProtocolViolation(message))
                }
                other => Err(other),
            };
        }
    }

    Ok(())
}

fn execute_commit_request_if_present(
    runtime: &OrbitRuntime,
    result: &Value,
) -> Result<(), OrbitError> {
    let Some(commit_value) = result.get("commit") else {
        return Ok(());
    };

    let commit: AgentCommitRequest =
        serde_json::from_value(commit_value.clone()).map_err(|error| {
            OrbitError::AgentProtocolViolation(format!(
                "result.commit must be an object with string `message` and string-array `files`: {error}"
            ))
        })?;

    if commit.message.trim().is_empty() {
        return Err(OrbitError::AgentProtocolViolation(
            "result.commit.message must not be empty".to_string(),
        ));
    }
    if commit.files.is_empty() {
        return Err(OrbitError::AgentProtocolViolation(
            "result.commit.files must contain at least one path".to_string(),
        ));
    }
    let files = commit.files.clone();
    let message = commit.message.clone();

    let repo_root = paths::find_git_repo_root(runtime.data_root_path()).ok_or_else(|| {
        OrbitError::Execution(format!(
            "cannot locate git repository root from Orbit data root '{}'",
            runtime.data_root_path().display()
        ))
    })?;
    let repo_root_str = repo_root.to_string_lossy().to_string();

    runtime.run_tool(
        "git.stage_paths",
        json!({
            "repo_root": repo_root_str,
            "files": files.clone(),
        }),
    )?;
    runtime.run_tool(
        "git.commit",
        json!({
            "repo_root": repo_root.to_string_lossy(),
            "message": message,
            "files": files,
        }),
    )?;
    Ok(())
}

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

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        OrbitRuntime::validate_activity_target_exists(self, target_type, target_id)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
        self.get_job_record(job_id)
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
        let tasks = self.list_task_records()?;
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
        let plan = format!(
            "1. Investigate the root cause for job `{job_id}` failure (error code: `{error_code}`)\n\
             2. Fix the underlying issue\n\
             3. Re-run the job to verify it completes successfully"
        );
        let _ = self.add_task_with_identity(
            crate::command::task::TaskAddParams {
                parent_id: None,
                title,
                description,
                plan,
                comment: None,
                context_files: vec![],
                workspace_path: None,
                priority: TaskPriority::High,
                complexity: None,
                task_type: TaskType::Friction,
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
}

impl JobRunHost for OrbitRuntime {
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_pending_or_running_job_runs_record(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.insert_job_run_record(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.mark_job_run_running_record(run_id, started_at, pid)
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.abandon_job_run_record(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.complete_job_run_step_record(run_id, params)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.finalize_job_run_record(run_id, state, finished_at, duration_ms)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_job_run_record(run_id)
    }
}

impl EnvironmentHost for OrbitRuntime {
    fn agent_config_for(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Result<AgentConfig, OrbitError> {
        let provider_options = ProviderOptions::for_agent_cli(
            agent_cli,
            self.codex_execution_policy().sandbox().to_string(),
            self.codex_execution_policy()
                .approval_policy()
                .map(|s| s.to_string()),
        )?;
        Ok(AgentConfig {
            command: agent_cli.to_string(),
            model: model.map(|m| m.to_string()),
            provider_options,
        })
    }

    fn execution_environment_mode(&self, env_extra: &[String]) -> EnvironmentMode {
        if self.execution_env_policy().inherit() {
            // Full inheritance: ORBIT_ROOT resolution relies on find_git_repo_root
            // handling git worktrees correctly (which it does after the worktree fix).
            EnvironmentMode::Inherit
        } else {
            let mut env = self
                .execution_env_policy()
                .hydrated_allowlist_env_with_extras(env_extra);
            // Explicitly inject ORBIT_ROOT so that any orbit CLI invocation made
            // by the agent subprocess (e.g., `orbit tool run orbit.task.*`) resolves
            // to the same data root regardless of its working directory. Without this,
            // a Codex or Claude agent running inside a git worktree would either create
            // a spurious .orbit/ in the worktree or resolve to the wrong database.
            let orbit_root = self.data_root_path().to_string_lossy().into_owned();
            if !env.iter().any(|(k, _)| k == "ORBIT_ROOT") {
                env.push(("ORBIT_ROOT".to_string(), orbit_root));
            }
            EnvironmentMode::ClearAndSet(env)
        }
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.execution_env_policy()
            .hydrated_cli_command_env_with_extras(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.execution_env_policy()
            .missing_required(required_env_vars)
    }
}

impl AgentProtocolHost for OrbitRuntime {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        build_agent_stdin_envelope_payload(self, execution)
    }

    fn validate_skill_output_schema(
        &self,
        activity: &Activity,
        envelope: &AgentResponseEnvelope,
    ) -> Result<(), OrbitError> {
        validate_skill_output_schema(self, activity, envelope)
    }

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
        execute_commit_request_if_present(self, result)
    }
}

impl TaskHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::start_task(self, task_id, note, comment)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::update_task_from_activity(
            self,
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
        let _ = self.with_mutation(|| {
            let task = self.update_task_record(
                task_id,
                StoreTaskUpdateParams {
                    actor: "agent".to_string(),
                    execution_summary: update.execution_summary.clone(),
                    status: update.status,
                    workspace_path: update.workspace_path.clone().map(Some),
                    repo_root: update.repo_root.clone().map(Some),
                    pr_number: update.pr_number.clone().map(Some),
                    agent: update.agent.clone().map(Some),
                    model: update.model.clone().map(Some),
                    ..Default::default()
                },
            )?;
            Ok((
                task.clone(),
                OrbitEvent::TaskUpdated {
                    id: task_id.to_string(),
                },
            ))
        })?;
        Ok(())
    }
}

fn activity_envelope_json(activity: &Activity) -> Value {
    let mut envelope = json!({
        "id": activity.id,
        "type": activity.spec_type,
        "description": activity.description,
        "input_schema_json": activity.input_schema_json,
        "output_schema_json": activity.output_schema_json,
        "created_by": activity.created_by,
    });

    if let Some(activity_map) = envelope.as_object_mut()
        && let Some(spec_config) = activity.spec_config.as_object()
    {
        for (key, value) in spec_config {
            activity_map.insert(key.clone(), value.clone());
        }
    }

    envelope
}

fn current_repo_root(runtime: &OrbitRuntime) -> Result<String, OrbitError> {
    let repo_root = paths::find_git_repo_root(runtime.data_root_path()).ok_or_else(|| {
        OrbitError::Execution(format!(
            "cannot locate git repository root from Orbit data root '{}'",
            runtime.data_root_path().display()
        ))
    })?;
    Ok(repo_root.to_string_lossy().to_string())
}
