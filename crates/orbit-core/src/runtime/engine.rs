use chrono::{DateTime, Utc};
use orbit_engine::{
    ActivityInvocationResult, AgentProtocolHost, EnvironmentHost, ExecutionContext, JobRunHost,
    RuntimeHost, TaskAutomationUpdate, TaskHost, activity_skill_refs_from_spec_config,
    execute_single_attempt, validate_activity_input_schema,
};
use orbit_store::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationInsertParams, InvocationQuery,
    InvocationRecord, JobRunStepParams, Store, TaskInvocationMetrics,
    TaskUpdateParams as StoreTaskUpdateParams, ToolInvocationMetrics, token_scoreboard,
};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, ActorIdentity, AgentCommitRequest, AgentModelPair, InvocationTrace, JobRun,
    JobRunState, JobTargetType, KnowledgeRunMetrics, OrbitError, OrbitEvent, Role, Task,
    TaskPriority, TaskStatus, TaskType, WorkspacePaths, agent_family_from_cli,
    prune_missing_context_files, resolve_agent_model_pair,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use crate::OrbitRuntime;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<Value>,
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
    let task = task_detail_for_input(
        runtime,
        &execution.input,
        &runtime.context.paths().repo_root,
    )?;
    let envelope = ExecutionEnvelope {
        schema_version: 1,
        activity: activity_envelope_json_for_execution_with_pair(
            &execution.activity,
            &execution.agent_cli,
            runtime.configured_agent_model_pair(&execution.agent_cli),
        ),
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
        task,
    };

    serde_json::to_vec(&envelope)
        .map_err(|e| OrbitError::Execution(format!("failed to serialize stdin envelope: {e}")))
}

fn task_detail_for_input<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
    fallback_repo_root: &Path,
) -> Result<Option<Value>, OrbitError> {
    let Some(task_id) = input.get("task_id").and_then(Value::as_str) else {
        return Ok(None);
    };

    let task = host.get_task(task_id)?;
    Ok(Some(task_detail_envelope_json(
        &task,
        input,
        fallback_repo_root,
    )))
}

fn task_detail_envelope_json(task: &Task, input: &Value, fallback_repo_root: &Path) -> Value {
    let workspace_path = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| task.workspace_path.clone());
    let repo_root = input
        .get("repo_root")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| task.repo_root.clone());

    // Read-time safety net: drop any `context_files` entries whose resolved
    // paths no longer exist on disk. The authoritative fix lives at write-time
    // in orbit-core, but files can be deleted *after* a task is written, and
    // existing tasks on disk may still reference stale paths. Keep the on-disk
    // task untouched — this only filters what reaches the agent envelope.
    let prune_root: PathBuf = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_repo_root.to_path_buf());
    let (kept_context_files, _dropped) =
        prune_missing_context_files(&prune_root, task.context_files.clone());

    json!({
        "id": task.id.clone(),
        "title": task.title.clone(),
        "description": task.description.clone(),
        "acceptance_criteria": task.acceptance_criteria.clone(),
        "plan": task.plan.clone(),
        "context_files": kept_context_files,
        "pr_number": task.pr_number.clone(),
        "workspace_path": workspace_path,
        "repo_root": repo_root,
    })
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

    let repo_root = &runtime.context.paths().repo_root;
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

fn open_invocation_store(runtime: &OrbitRuntime) -> Result<Store, OrbitError> {
    Store::open(&runtime.context.persistence().audit_db)
}

fn normalize_agent_name(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

fn associated_task_ids(input: &Value) -> Vec<String> {
    let mut task_ids = Vec::new();
    if let Some(task_id) = input.get("task_id").and_then(Value::as_str) {
        push_unique_task_id(&mut task_ids, task_id);
    }
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    if let Some(items) = input.get("tasks").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
                continue;
            }
            if let Some(task_id) = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("task_id").and_then(Value::as_str))
            {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    task_ids
}

fn push_unique_task_id(task_ids: &mut Vec<String>, task_id: &str) {
    let task_id = task_id.trim();
    if !task_id.is_empty() && !task_ids.iter().any(|existing| existing == task_id) {
        task_ids.push(task_id.to_string());
    }
}

impl OrbitRuntime {
    pub fn activity_invocation_metrics(
        &self,
    ) -> Result<Vec<ActivityInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_activity_invocation_metrics()
    }

    pub fn agent_invocation_metrics(&self) -> Result<Vec<AgentInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_agent_invocation_metrics()
    }

    pub fn task_invocation_metrics(
        &self,
        task_id: &str,
    ) -> Result<TaskInvocationMetrics, OrbitError> {
        open_invocation_store(self)?.get_task_invocation_metrics(task_id)
    }

    pub fn tool_invocation_metrics(&self) -> Result<Vec<ToolInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_tool_invocation_metrics()
    }

    pub(crate) fn configured_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.context
            .agent_model_pair(agent_cli)
            .or_else(|| resolve_agent_model_pair(agent_cli))
    }

    pub(crate) fn canonical_model_for_agent(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Option<String> {
        self.context.canonical_model_name(agent_cli, model)
    }

    pub(crate) fn canonical_agent_model_identity(
        &self,
        agent_cli: Option<&str>,
        model: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let agent = agent_cli
            .map(normalize_agent_name)
            .filter(|value| !value.trim().is_empty());
        let model = agent
            .as_deref()
            .and_then(|agent| self.canonical_model_for_agent(agent, model))
            .or_else(|| {
                model
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            });
        (agent, model)
    }

    pub(crate) fn ship_role_assignment_for(&self, role: &str) -> Option<(String, String)> {
        self.context
            .ship_role_assignment(role)
            .map(|assignment| (assignment.agent, assignment.model))
    }

    pub fn generate_scoreboard_summary(
        &self,
    ) -> Result<orbit_store::scoreboard_summary::ScoreboardSummary, OrbitError> {
        let tasks = self.list_tasks()?;
        let summary = orbit_store::scoreboard_summary::generate_summary(
            &self.paths().scoreboard_dir,
            &tasks,
        )?;
        let _ =
            orbit_store::scoreboard_summary::write_summary(&self.paths().scoreboard_dir, &summary)?;
        Ok(summary)
    }

    pub fn scoreboard_summary_path(&self) -> std::path::PathBuf {
        orbit_store::scoreboard_summary::summary_path(&self.paths().scoreboard_dir)
    }

    pub fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        open_invocation_store(self)?.list_invocation_records(&query)
    }

    /// Read the append-only duel scoreboard log. The CLI's
    /// `orbit duel scoreboard` command aggregates the returned runs in
    /// memory via `orbit_store::duel_scoreboard::aggregate`. Returns an
    /// empty vector when the file does not yet exist — an unrun
    /// scoreboard is not an error condition.
    pub fn load_duel_runs(&self) -> Result<Vec<orbit_types::DuelRun>, OrbitError> {
        orbit_store::duel_scoreboard::load_runs(&self.paths().scoreboard_dir)
    }
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
        self.get_job_record(job_id)
    }

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.configured_agent_model_pair(agent_cli)
    }

    fn canonical_model_name(&self, agent_cli: &str, model: Option<&str>) -> Option<String> {
        self.canonical_model_for_agent(agent_cli, model)
    }

    fn ship_role_assignment(&self, role: &str) -> Option<(String, String)> {
        self.ship_role_assignment_for(role)
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
            timeout_seconds,
            env_extra: vec![],
            env_set: std::collections::HashMap::new(),
            input,
            debug,
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

impl JobRunHost for OrbitRuntime {
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_pending_or_running_job_runs_record(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.insert_job_run_record(job_id, attempt, scheduled_at, input, retry_source_run_id)
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

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.record_job_run_knowledge_metrics_record(run_id, metrics)
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
    fn agent_provider_config(&self) -> std::collections::HashMap<String, String> {
        let mut config = std::collections::HashMap::new();
        let policy = self.codex_execution_policy();
        config.insert("sandbox".to_string(), policy.sandbox().to_string());
        if let Some(approval) = policy.approval_policy() {
            config.insert("approval_policy".to_string(), approval.to_string());
        }
        if policy.sandbox() == "workspace-write" {
            config.insert(
                "writable_dirs_json".to_string(),
                serde_json::to_string(&codex_workspace_write_writable_dirs(self.context.paths()))
                    .unwrap_or_else(|_| "[]".to_string()),
            );
        }
        config
    }

    fn execution_env_inherit(&self) -> bool {
        self.execution_env_policy().inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.execution_env_policy()
            .hydrated_allowlist_env_with_extras(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        Some(
            self.context
                .paths()
                .orbit_dir
                .to_string_lossy()
                .into_owned(),
        )
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

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
        execute_commit_request_if_present(self, result)
    }
}

impl TaskHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn get_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
        OrbitRuntime::get_task_artifacts(self, task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        OrbitRuntime::list_tasks_filtered(self, status, priority, parent_id, batch_id)
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
        if update.status == Some(TaskStatus::InProgress) {
            let task = self.get_task(task_id)?;
            if crate::command::task::in_progress_transition_requires_plan(task.status) {
                crate::command::task::ensure_task_has_execution_plan(task_id, task.plan.as_str())?;
            }
        }
        let _ = self.with_mutation(|| {
            let (agent, model) = self
                .canonical_agent_model_identity(update.agent.as_deref(), update.model.as_deref());
            let task = self.update_task_record(
                task_id,
                StoreTaskUpdateParams {
                    actor: "agent".to_string(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    status: update.status,
                    workspace_path: update.workspace_path.clone(),
                    repo_root: update.repo_root.clone().map(Some),
                    pr_number: update.pr_number.clone().map(Some),
                    batch_id: update.batch_id.clone().map(Some),
                    status_event: update.status_event.clone(),
                    status_note: update.status_note.clone(),
                    append_comments: update.append_comments.clone(),
                    actor_identity: Some(ActorIdentity::from_legacy(
                        agent.as_deref(),
                        model.as_deref(),
                    ))
                    .filter(|id| !id.is_system()),
                    replace_review_threads: update.review_threads.clone(),
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

#[cfg(test)]
fn activity_envelope_json(activity: &Activity) -> Value {
    activity_envelope_json_for_execution(activity, "")
}

fn activity_envelope_json_for_execution_with_pair(
    activity: &Activity,
    agent_cli: &str,
    pair: Option<AgentModelPair>,
) -> Value {
    let pair = pair.or_else(|| resolve_agent_model_pair(agent_cli));
    let family = agent_family_from_cli(agent_cli);
    let orchestrator = pair.as_ref().map(|p| p.orchestrator.as_str()).unwrap_or("");
    let helper = pair.as_ref().map(|p| p.helper.as_str()).unwrap_or("");

    let mut envelope = json!({
        "id": activity.id,
        "type": activity.spec_type,
        "description": activity.description,
        "input_schema_json": activity.input_schema_json,
        "created_by": activity.created_by,
    });

    if let Some(activity_map) = envelope.as_object_mut()
        && let Some(spec_config) = activity.spec_config.as_object()
    {
        for (key, value) in spec_config {
            activity_map.insert(key.clone(), value.clone());
        }
    }

    if let Some(activity_map) = envelope.as_object_mut() {
        activity_map.insert("agent_family".to_string(), json!(family));
        activity_map.insert("orchestrator_model".to_string(), json!(orchestrator));
        activity_map.insert("helper_model".to_string(), json!(helper));

        if let Some(instruction_value) = activity_map.get("instruction").cloned()
            && let Some(instruction_str) = instruction_value.as_str()
        {
            let rendered = render_agent_pair_placeholders(instruction_str, &family, &pair);
            activity_map.insert("instruction".to_string(), Value::String(rendered));
        }
    }

    envelope
}

/// Build the activity envelope JSON, embedding the orchestrator/helper model
/// pair resolved from `agent_cli` and substituting `{{orchestrator_model}}`,
/// `{{helper_model}}`, and `{{agent_family}}` placeholders inside the
/// activity's `instruction` text.
#[cfg(test)]
fn activity_envelope_json_for_execution(activity: &Activity, agent_cli: &str) -> Value {
    activity_envelope_json_for_execution_with_pair(activity, agent_cli, None)
}

/// Substitute the orchestrator/helper placeholders inside an instruction
/// string. Falls back to descriptive sentinels when no mapping exists for the
/// configured agent family so unknown families do not silently render the
/// placeholder tokens to the agent.
fn render_agent_pair_placeholders(
    instruction: &str,
    family: &str,
    pair: &Option<AgentModelPair>,
) -> String {
    let family_value = if family.is_empty() {
        "(unspecified)".to_string()
    } else {
        family.to_string()
    };
    let (orchestrator_value, helper_value) = match pair {
        Some(pair) => (pair.orchestrator.clone(), pair.helper.clone()),
        None => (
            format!("(no orchestrator mapping for {family_value})"),
            format!("(no helper mapping for {family_value})"),
        ),
    };

    instruction
        .replace("{{agent_family}}", &family_value)
        .replace("{{orchestrator_model}}", &orchestrator_value)
        .replace("{{helper_model}}", &helper_value)
}

fn current_repo_root(runtime: &OrbitRuntime) -> Result<String, OrbitError> {
    Ok(runtime
        .context
        .paths()
        .repo_root
        .to_string_lossy()
        .to_string())
}

fn codex_workspace_write_writable_dirs(paths: &WorkspacePaths) -> Vec<String> {
    let mut dirs = Vec::new();
    for dir in [&paths.orbit_dir, &paths.global_dir] {
        let dir = dir.to_string_lossy().into_owned();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use chrono::Utc;
    use orbit_engine::{RuntimeHost, TaskAutomationUpdate, TaskHost};
    use orbit_types::{
        Activity, ActorIdentity, InvocationTrace, JobRunState, JobScheduleState, JobStep,
        JobTargetType, OrbitError, ReviewThread, Task, TaskComment, TaskPriority, TaskStatus,
        TaskType, TokenUsage, ToolCallTrace, WorkspacePaths,
    };
    use serde_json::json;

    use super::{
        ExecutionEnvelope, ExecutionSkillEnvelope, activity_envelope_json,
        activity_envelope_json_for_execution, build_agent_stdin_envelope_payload,
        codex_workspace_write_writable_dirs, task_detail_envelope_json, task_detail_for_input,
    };
    use crate::OrbitRuntime;
    use crate::command::{activity::ActivityAddParams, job::JobAddParams, task::TaskAddParams};

    fn runtime_from_config(config_toml: &str) -> (tempfile::TempDir, OrbitRuntime) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let global_root = tmp.path().join("global/.orbit");
        let workspace_root = tmp.path().join("workspace/.orbit");
        fs::create_dir_all(&global_root).expect("global root");
        fs::create_dir_all(&workspace_root).expect("workspace root");
        fs::write(workspace_root.join("config.toml"), config_toml).expect("config");
        (
            tmp,
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("runtime"),
        )
    }

    struct MockTaskHost {
        task: Task,
    }

    impl orbit_engine::TaskHost for MockTaskHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            if self.task.id == task_id {
                Ok(self.task.clone())
            } else {
                Err(OrbitError::TaskNotFound(task_id.to_string()))
            }
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            unreachable!("not used in this test")
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unreachable!("not used in this test")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unreachable!("not used in this test")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            unreachable!("not used in this test")
        }
    }

    fn sample_task() -> Task {
        let now = Utc::now();
        Task {
            id: "T20260408-0133".to_string(),
            parent_id: None,
            title: "Inject task details".to_string(),
            description: "Populate task details in the agent envelope.".to_string(),
            acceptance_criteria: vec!["task field is present".to_string()],
            plan: "1. Inject task detail\n2. Update tests".to_string(),
            execution_summary: String::new(),
            context_files: vec![
                "crates/orbit-core/src/runtime/engine.rs".to_string(),
                "crates/orbit-core/assets/activities/agent_invoke/implement_change.yaml"
                    .to_string(),
            ],
            workspace_path: Some("/task/worktree".to_string()),
            repo_root: Some("/task/repo".to_string()),
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::High,
            complexity: None,
            task_type: TaskType::Feature,
            pr_number: Some("42".to_string()),
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: None,
            comments: vec![],
            history: vec![],
            review_threads: Vec::<ReviewThread>::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_activity() -> Activity {
        let now = Utc::now();
        Activity {
            id: "implement_change".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "test activity".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({}),
            tools: vec!["orbit.task.update".to_string()],
            proc_allowed_programs: vec!["cargo".to_string()],
            workspace_path: None,
            created_by: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn add_automation_activity(runtime: &OrbitRuntime, id: &str, action: &str) {
        runtime
            .add_activity(ActivityAddParams {
                id: id.to_string(),
                spec_type: "automation".to_string(),
                description: format!("automation activity {id}"),
                input_schema_json: json!({}),
                output_schema_json: json!({}),
                spec_config: json!({ "action": action }),
                workspace_path: None,
                created_by: None,
            })
            .expect("activity");
    }

    fn add_single_step_job(
        runtime: &OrbitRuntime,
        job_id: &str,
        activity_id: &str,
        max_iterations: u32,
    ) {
        runtime
            .add_job(JobAddParams {
                job_id: Some(job_id.to_string()),
                default_input: None,
                max_active_runs: Some(1),
                max_iterations: Some(max_iterations),
                steps: vec![JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: activity_id.to_string(),
                    timeout_seconds: 30,
                    ..JobStep::default()
                }],
                initial_state_override: Some(JobScheduleState::Enabled),
            })
            .expect("job");
    }

    fn blocked_history_note(task: &Task) -> &str {
        task.history
            .iter()
            .find(|entry| entry.to_status == Some(TaskStatus::Blocked))
            .and_then(|entry| entry.note.as_deref())
            .expect("blocked history note")
    }

    #[test]
    fn workspace_write_includes_workspace_and_global_orbit_dirs() {
        let paths = WorkspacePaths::new(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.orbit"),
            PathBuf::from("/Users/test/.orbit"),
        );

        assert_eq!(
            codex_workspace_write_writable_dirs(&paths),
            vec!["/repo/.orbit".to_string(), "/Users/test/.orbit".to_string(),]
        );
    }

    #[test]
    fn workspace_write_deduplicates_identical_orbit_dirs() {
        let paths = WorkspacePaths::new(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.orbit"),
            PathBuf::from("/repo/.orbit"),
        );

        assert_eq!(
            codex_workspace_write_writable_dirs(&paths),
            vec!["/repo/.orbit".to_string()]
        );
    }

    #[test]
    fn task_detail_envelope_prefers_input_overrides() {
        let detail = task_detail_envelope_json(
            &sample_task(),
            &json!({
                "workspace_path": "/override/worktree",
                "repo_root": "/override/repo",
            }),
            std::path::Path::new("/"),
        );

        assert_eq!(
            detail.get("workspace_path"),
            Some(&json!("/override/worktree"))
        );
        assert_eq!(detail.get("repo_root"), Some(&json!("/override/repo")));
    }

    #[test]
    fn task_detail_envelope_falls_back_to_task_paths() {
        let detail =
            task_detail_envelope_json(&sample_task(), &json!({}), std::path::Path::new("/"));

        assert_eq!(detail.get("id"), Some(&json!("T20260408-0133")));
        assert_eq!(detail.get("pr_number"), Some(&json!("42")));
        assert_eq!(detail.get("workspace_path"), Some(&json!("/task/worktree")));
        assert_eq!(detail.get("repo_root"), Some(&json!("/task/repo")));
    }

    #[test]
    fn task_detail_envelope_prunes_missing_context_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("real.md"), "hi").expect("write real.md");

        let mut task = sample_task();
        task.workspace_path = Some(tmp.path().to_string_lossy().into_owned());
        task.context_files = vec![
            "real.md".to_string(),
            "ghost.md".to_string(),
            "another/missing.md".to_string(),
        ];

        let detail = task_detail_envelope_json(&task, &json!({}), std::path::Path::new("/"));

        let kept = detail
            .get("context_files")
            .and_then(serde_json::Value::as_array)
            .expect("context_files array");
        let kept_strings: Vec<&str> = kept.iter().filter_map(serde_json::Value::as_str).collect();
        assert_eq!(kept_strings, vec!["real.md"]);
    }

    #[test]
    fn task_detail_for_input_returns_none_without_task_id() {
        let host = MockTaskHost {
            task: sample_task(),
        };

        let detail = task_detail_for_input(&host, &json!({}), std::path::Path::new("/"))
            .expect("task detail");

        assert!(detail.is_none());
    }

    #[test]
    fn serialized_execution_envelope_includes_task_details_when_present() {
        let host = MockTaskHost {
            task: sample_task(),
        };
        let input = json!({
            "task_id": "T20260408-0133",
            "workspace_path": "/override/worktree",
        });
        let task =
            task_detail_for_input(&host, &input, std::path::Path::new("/")).expect("task detail");

        let envelope = ExecutionEnvelope {
            schema_version: 1,
            activity: activity_envelope_json(&sample_activity()),
            job: None,
            skills: vec![ExecutionSkillEnvelope {
                id: "orbit".to_string(),
                content_hash: "hash".to_string(),
                content: "content".to_string(),
                meta: None,
            }],
            input,
            memory: json!({}),
            task,
        };

        let serialized = serde_json::to_value(&envelope).expect("serialized envelope");

        assert_eq!(
            serialized.get("task").and_then(|task| task.get("title")),
            Some(&json!("Inject task details"))
        );
        assert_eq!(
            serialized
                .get("task")
                .and_then(|task| task.get("pr_number")),
            Some(&json!("42"))
        );
        assert_eq!(
            serialized
                .get("task")
                .and_then(|task| task.get("workspace_path")),
            Some(&json!("/override/worktree"))
        );
    }

    fn implement_change_sample_activity() -> Activity {
        let now = Utc::now();
        Activity {
            id: "implement_change".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "implement an Orbit task".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Roles: orchestrator={{orchestrator_model}}, helper={{helper_model}}, family={{agent_family}}",
            }),
            tools: vec![],
            proc_allowed_programs: vec![],
            workspace_path: None,
            created_by: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn activity_envelope_renders_codex_orchestrator_helper_pair() {
        let envelope =
            activity_envelope_json_for_execution(&implement_change_sample_activity(), "codex");
        assert_eq!(envelope.get("agent_family"), Some(&json!("codex")));
        assert_eq!(envelope.get("orchestrator_model"), Some(&json!("gpt-5.4")));
        assert_eq!(envelope.get("helper_model"), Some(&json!("gpt-5.4-mini")));
        let instruction = envelope
            .get("instruction")
            .and_then(serde_json::Value::as_str)
            .expect("instruction");
        assert!(instruction.contains("orchestrator=gpt-5.4"));
        assert!(instruction.contains("helper=gpt-5.4-mini"));
        assert!(instruction.contains("family=codex"));
        assert!(!instruction.contains("{{"));
    }

    #[test]
    fn activity_envelope_renders_claude_orchestrator_helper_pair() {
        let envelope = activity_envelope_json_for_execution(
            &implement_change_sample_activity(),
            "/usr/local/bin/claude",
        );
        assert_eq!(envelope.get("agent_family"), Some(&json!("claude")));
        assert_eq!(envelope.get("orchestrator_model"), Some(&json!("opus-4.6")));
        assert_eq!(envelope.get("helper_model"), Some(&json!("sonnet-4.6")));
        let instruction = envelope
            .get("instruction")
            .and_then(serde_json::Value::as_str)
            .expect("instruction");
        assert!(instruction.contains("orchestrator=opus-4.6"));
        assert!(instruction.contains("helper=sonnet-4.6"));
        assert!(instruction.contains("family=claude"));
    }

    #[test]
    fn agent_stdin_envelope_uses_configured_agent_pair_placeholders() {
        let (_tmp, runtime) = runtime_from_config(
            r#"
[agents.claude]
strong = "opus-4.7"
weak = "sonnet-4.7"
"#,
        );
        let execution = orbit_engine::ExecutionContext {
            activity: implement_change_sample_activity(),
            job: None,
            agent_cli: "claude".to_string(),
            model: Some("opus".to_string()),
            timeout_seconds: 30,
            env_extra: vec![],
            env_set: std::collections::HashMap::new(),
            input: json!({}),
            debug: false,
        };

        let payload = build_agent_stdin_envelope_payload(&runtime, &execution).expect("payload");
        let envelope: serde_json::Value = serde_json::from_slice(&payload).expect("envelope json");
        let activity = &envelope["activity"];

        assert_eq!(activity["orchestrator_model"], json!("opus-4.7"));
        assert_eq!(activity["helper_model"], json!("sonnet-4.7"));
        let instruction = activity["instruction"].as_str().expect("instruction");
        assert!(instruction.contains("orchestrator=opus-4.7"));
        assert!(instruction.contains("helper=sonnet-4.7"));
    }

    #[test]
    fn activity_envelope_renders_gemini_orchestrator_helper_pair() {
        let envelope =
            activity_envelope_json_for_execution(&implement_change_sample_activity(), "gemini");
        assert_eq!(envelope.get("agent_family"), Some(&json!("gemini")));
        assert_eq!(
            envelope.get("orchestrator_model"),
            Some(&json!("gemini-3.1-pro-preview"))
        );
        assert_eq!(
            envelope.get("helper_model"),
            Some(&json!("gemini-3-flash-preview"))
        );
        let instruction = envelope
            .get("instruction")
            .and_then(serde_json::Value::as_str)
            .expect("instruction");
        assert!(instruction.contains("orchestrator=gemini-3.1-pro-preview"));
        assert!(instruction.contains("helper=gemini-3-flash-preview"));
    }

    #[test]
    fn activity_envelope_unknown_agent_family_emits_sentinel_text() {
        let envelope =
            activity_envelope_json_for_execution(&implement_change_sample_activity(), "mock-agent");
        assert_eq!(envelope.get("orchestrator_model"), Some(&json!("")));
        let instruction = envelope
            .get("instruction")
            .and_then(serde_json::Value::as_str)
            .expect("instruction");
        // The placeholders must still be substituted (no raw template tokens)
        // even when no mapping is registered.
        assert!(!instruction.contains("{{"));
        assert!(instruction.contains("no orchestrator mapping for mock-agent"));
        assert!(instruction.contains("no helper mapping for mock-agent"));
    }

    #[test]
    fn activity_envelope_without_instruction_still_injects_pair_fields() {
        let mut activity = implement_change_sample_activity();
        activity.spec_config = json!({});
        let envelope = activity_envelope_json_for_execution(&activity, "codex");
        assert_eq!(envelope.get("orchestrator_model"), Some(&json!("gpt-5.4")));
        assert!(envelope.get("instruction").is_none());
    }

    #[test]
    fn apply_task_automation_update_persists_plan_history_and_commentary() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_identity(
                TaskAddParams {
                    title: "Planning duel".to_string(),
                    description: "Test planning-duel writeback".to_string(),
                    acceptance_criteria: vec!["persist winning plan".to_string()],
                    plan: "old plan".to_string(),
                    comment: None,
                    context_files: vec![],
                    workspace_path: None,
                    priority: orbit_types::TaskPriority::Medium,
                    complexity: None,
                    task_type: orbit_types::TaskType::Task,
                    source_task_id: None,
                    ..Default::default()
                },
                Some("planner-a".to_string()),
                Some("model-a".to_string()),
            )
            .expect("task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    plan: Some("winning plan".to_string()),
                    status: Some(TaskStatus::Backlog),
                    status_event: Some("planning_duel_resolved".to_string()),
                    status_note: Some("winner=planner_a".to_string()),
                    append_comments: vec![TaskComment {
                        at: Utc::now(),
                        by: "arbiter".to_string(),
                        message: "arbiter rationale".to_string(),
                    }],
                    agent: Some("planner-a".to_string()),
                    model: Some("model-a".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("update");

        let updated = runtime.get_task(&task.id).expect("updated task");
        assert_eq!(updated.plan, "winning plan");
        assert_eq!(updated.status, TaskStatus::Backlog);
        assert_eq!(updated.actor_identity.agent_name(), Some("planner-a"));
        assert_eq!(updated.actor_identity.agent_model(), Some("model-a"));
        assert_eq!(updated.comments.len(), 1);
        assert_eq!(updated.comments[0].by, "arbiter");
        assert_eq!(updated.comments[0].message, "arbiter rationale");
        assert!(
            updated
                .history
                .iter()
                .any(|entry| entry.event == "planning_duel_resolved"
                    && entry.note.as_deref() == Some("winner=planner_a"))
        );
    }

    #[test]
    fn maybe_create_failure_task_uses_issue_type() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");

        runtime
            .maybe_create_failure_task(
                "job_fail_issue_type",
                "run-123",
                "ACTIVITY_EXECUTION_FAILED",
                "unsupported automation action",
                Some("codex"),
                Some("gpt-5.4"),
            )
            .expect("failure task");

        let task = runtime
            .list_tasks()
            .expect("tasks")
            .into_iter()
            .find(|task| {
                task.title == "Job failure: job_fail_issue_type [ACTIVITY_EXECUTION_FAILED]"
            })
            .expect("created failure task");

        assert_eq!(task.task_type, TaskType::Issue);
        assert!(!task.task_type.counts_toward_friction_bounty());
        assert_eq!(task.created_by, Some("system".to_string()));
        assert_eq!(task.proposed_by, Some("system".to_string()));
        assert_eq!(task.assigned_to, Some("system".to_string()));
        assert_eq!(task.actor_identity.agent_name(), Some("codex"));
        assert_eq!(task.actor_identity.agent_model(), Some("gpt-5.4"));
    }

    #[test]
    fn maybe_create_failure_task_dedupes_open_task() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");

        for _ in 0..2 {
            runtime
                .maybe_create_failure_task(
                    "job_fail_dedupe",
                    "run-123",
                    "ACTIVITY_EXECUTION_FAILED",
                    "unsupported automation action",
                    Some("codex"),
                    Some("gpt-5.4"),
                )
                .expect("failure task");
        }

        let matching_tasks = runtime
            .list_tasks()
            .expect("tasks")
            .into_iter()
            .filter(|task| task.title == "Job failure: job_fail_dedupe [ACTIVITY_EXECUTION_FAILED]")
            .count();

        assert_eq!(matching_tasks, 1);
    }

    #[test]
    fn failed_task_scoped_job_run_blocks_task_with_failure_history() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("workflow failure", TaskStatus::InProgress)
            .expect("task");
        add_automation_activity(&runtime, "failing_automation", "unsupported_action");
        add_single_step_job(&runtime, "job_fail_task_scoped", "failing_automation", 1);

        let result = runtime
            .run_job_now_with_input(
                "job_fail_task_scoped",
                json!({
                    "task_id": task.id,
                }),
            )
            .expect("job result");

        assert_eq!(result.state, JobRunState::Failed);

        let updated = runtime.get_task(&task.id).expect("updated task");
        assert_eq!(updated.status, TaskStatus::Blocked);
        let blocked_entry = updated
            .history
            .iter()
            .find(|entry| entry.to_status == Some(TaskStatus::Blocked))
            .expect("blocked history entry");
        assert_eq!(blocked_entry.event, "workflow_run_failed");
        let note = blocked_entry.note.as_deref().expect("blocked note");
        assert!(note.contains("job=job_fail_task_scoped"));
        assert!(note.contains(&format!("run_id={}", result.run_id)));
        assert!(note.contains("error_code=ACTIVITY_EXECUTION_FAILED"));
        assert!(note.contains("unsupported automation action 'unsupported_action'"));
    }

    #[test]
    fn loop_exhaustion_blocks_task_with_run_context_in_history() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("loop exhausted", TaskStatus::InProgress)
            .expect("task");
        add_automation_activity(&runtime, "successful_automation", "update_task");
        add_single_step_job(&runtime, "job_loop_exhaustion", "successful_automation", 2);

        let result = runtime
            .run_job_now_with_input(
                "job_loop_exhaustion",
                json!({
                    "task_id": task.id,
                }),
            )
            .expect("job result");

        assert_eq!(result.state, JobRunState::Failed);

        let updated = runtime.get_task(&task.id).expect("updated task");
        assert_eq!(updated.status, TaskStatus::Blocked);
        let note = blocked_history_note(&updated);
        assert!(note.contains("job=job_loop_exhaustion"));
        assert!(note.contains(&format!("run_id={}", result.run_id)));
        assert!(note.contains("error_code=LOOP_EXHAUSTED"));
        assert!(note.contains("loop exhausted after 2 iterations"));
    }

    #[test]
    fn invocation_records_for_job_run_and_activity_reads_persisted_trace() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let execution = orbit_engine::ExecutionContext {
            activity: sample_activity(),
            job: None,
            agent_cli: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
            timeout_seconds: 30,
            env_extra: vec![],
            env_set: std::collections::HashMap::new(),
            input: json!({
                "task_id": "T1"
            }),
            debug: false,
        };
        let trace = InvocationTrace {
            usage: TokenUsage {
                input: 12,
                cache_read: 1,
                cache_create: 2,
                output: 5,
            },
            tool_calls: vec![ToolCallTrace {
                seq: 1,
                tool_name: "orbit.graph.search".to_string(),
                result_bytes: 42,
                result_payload: None,
            }],
            duration_ms: 88,
        };

        runtime
            .persist_invocation_trace("run-123", &execution, &trace)
            .expect("persist trace");

        let records = runtime
            .invocation_records_for_job_run_and_activity("run-123", "implement_change")
            .expect("query records");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].job_run_id, "run-123");
        assert_eq!(records[0].activity_id, "implement_change");
        assert_eq!(records[0].tool_call_count, 1);
        assert_eq!(records[0].total_tokens, 17);
        assert_eq!(records[0].tool_calls[0].result_bytes, 42);
    }

    #[test]
    fn summary_uses_one_canonical_key_across_friction_and_token_sources() {
        let (_tmp, runtime) = runtime_from_config(
            r#"
[agents.claude]
strong = "opus-4.7"
weak = "sonnet-4.7"

[scoring]
enabled = true
"#,
        );

        runtime
            .add_task_with_identity(
                TaskAddParams {
                    title: "friction".to_string(),
                    description: "track canonical identity".to_string(),
                    acceptance_criteria: vec!["scoreboard writes normalize".to_string()],
                    plan: "## Plan\n- record".to_string(),
                    task_type: TaskType::Friction,
                    ..Default::default()
                },
                Some("claude".to_string()),
                Some("opus".to_string()),
            )
            .expect("task");

        let execution = orbit_engine::ExecutionContext {
            activity: sample_activity(),
            job: None,
            agent_cli: "claude".to_string(),
            model: Some("opus".to_string()),
            timeout_seconds: 30,
            env_extra: vec![],
            env_set: std::collections::HashMap::new(),
            input: json!({ "task_id": "T1" }),
            debug: false,
        };
        let trace = InvocationTrace {
            usage: TokenUsage {
                input: 12,
                cache_read: 0,
                cache_create: 0,
                output: 5,
            },
            tool_calls: vec![ToolCallTrace {
                seq: 1,
                tool_name: "orbit.graph.search".to_string(),
                result_bytes: 42,
                result_payload: None,
            }],
            duration_ms: 88,
        };
        runtime
            .persist_invocation_trace("run-123", &execution, &trace)
            .expect("persist trace");

        let summary = runtime.generate_scoreboard_summary().expect("summary");
        assert!(summary.agents.contains_key("claude/opus-4.7"));
        assert!(!summary.agents.contains_key("claude/opus"));
        assert_eq!(summary.agents["claude/opus-4.7"].friction.reported, 1);
        assert_eq!(summary.agents["claude/opus-4.7"].tokens.total, 17);
    }

    #[test]
    fn serialized_execution_envelope_omits_task_field_when_absent() {
        let envelope = ExecutionEnvelope {
            schema_version: 1,
            activity: activity_envelope_json(&sample_activity()),
            job: None,
            skills: vec![],
            input: json!({}),
            memory: json!({}),
            task: None,
        };

        let serialized = serde_json::to_value(&envelope).expect("serialized envelope");

        assert!(serialized.get("task").is_none());
    }
}
