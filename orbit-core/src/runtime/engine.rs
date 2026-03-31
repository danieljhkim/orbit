use chrono::{DateTime, Utc};
use orbit_engine::{
    AgentProtocolHost, EnvironmentHost, ExecutionContext, JobRunHost, RuntimeHost,
    TaskAutomationUpdate, TaskHost, activity_skill_refs_from_spec_config,
};
use orbit_store::{JobRunStepParams, TaskUpdateParams as StoreTaskUpdateParams};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, ActorIdentity, AgentCommitRequest, JobRun, JobRunState, JobTargetType, OrbitError,
    OrbitEvent, Role, Task, TaskPriority, TaskStatus, TaskType, WorkspacePaths,
};
use serde::Serialize;
use serde_json::{Value, json};

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

    fn acquire_file_locks(
        &self,
        task_id: &str,
        repo_root: &str,
        paths: &[&str],
    ) -> Result<(), OrbitError> {
        self.context
            .file_lock_store()
            .acquire_locks(task_id, repo_root, paths)
    }

    fn release_file_locks(&self, task_id: &str) -> Result<usize, OrbitError> {
        self.context
            .file_lock_store()
            .release_locks_for_task(task_id)
    }

    fn cleanup_stale_file_locks(&self) -> Result<usize, OrbitError> {
        let active_task_ids = self
            .list_task_records()?
            .into_iter()
            .filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::Review))
            .map(|task| task.id)
            .collect::<Vec<_>>();
        let active_refs = active_task_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.context
            .file_lock_store()
            .release_stale_locks(&active_refs)
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

    fn scoreboard_dir(&self) -> &std::path::Path {
        &self.context.paths().scoreboard_dir
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
            crate::command::task::ensure_task_has_execution_plan(task_id, task.plan.as_str())?;
        }
        let _ = self.with_mutation(|| {
            let task = self.update_task_record(
                task_id,
                StoreTaskUpdateParams {
                    actor: "agent".to_string(),
                    execution_summary: update.execution_summary.clone(),
                    status: update.status,
                    workspace_path: update.workspace_path.clone(),
                    repo_root: update.repo_root.clone().map(Some),
                    pr_number: update.pr_number.clone().map(Some),
                    batch_id: update.batch_id.clone().map(Some),
                    actor_identity: Some(ActorIdentity::from_legacy(
                        update.agent.as_deref(),
                        update.model.as_deref(),
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

fn activity_envelope_json(activity: &Activity) -> Value {
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

    envelope
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
    use std::path::PathBuf;

    use orbit_types::WorkspacePaths;

    use super::codex_workspace_write_writable_dirs;

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
}
