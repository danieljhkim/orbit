use chrono::{DateTime, Utc};
use orbit_agent::AgentConfig;
use orbit_engine::{
    EngineHost, ExecutionContext, TaskAutomationUpdate, activity_skill_refs_from_spec_config,
};
use orbit_exec::EnvironmentMode;
use orbit_store::JobRunStepParams;
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, AgentCommitRequest, AgentResponseEnvelope, JobRun, JobRunState, JobTargetType,
    OrbitError, OrbitEvent, Role, Task,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::TaskUpdateParams;
use crate::json_schema::validate_instance_against_schema;
use crate::paths;

#[derive(Debug, Clone, Serialize)]
struct ExecutionEnvelope {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    activity: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    job: Option<Value>,
    skills: Vec<ExecutionSkillEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<Value>,
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
    let identity = execution
        .activity
        .identity_id
        .as_deref()
        .map(|identity_id| runtime.resolve_identity(identity_id))
        .transpose()?
        .map(|resolved| {
            json!({
                "id": resolved.id,
                "name": resolved.name,
                "role": resolved.role.to_string(),
                "block": runtime.compile_identity_block(&resolved),
            })
        });
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
        identity,
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
    let skills = runtime.resolve_activity_skill_refs(&skill_refs)?;
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

    let repo_root = paths::find_git_repo_root(&runtime.context.data_root).ok_or_else(|| {
        OrbitError::Execution(format!(
            "cannot locate git repository root from Orbit data root '{}'",
            runtime.context.data_root.display()
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

impl EngineHost for OrbitRuntime {
    fn record_event(&self, event: OrbitEvent) -> Result<(), OrbitError> {
        OrbitRuntime::record_event(self, event)
    }

    fn validate_activity_target_exists(
        &self,
        target_type: JobTargetType,
        target_id: &str,
    ) -> Result<Activity, OrbitError> {
        OrbitRuntime::validate_activity_target_exists(self, target_type, target_id)
    }

    fn get_pending_or_running_job_run(&self, job_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.context
            .job_store
            .get_pending_or_running_job_run(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.context
            .job_store
            .insert_job_run(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store
            .mark_job_run_running(run_id, started_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.context.job_store.complete_job_run_step(run_id, params)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.context.job_store.get_job_run(run_id)
    }

    fn agent_config_for(&self, agent_cli: &str) -> Result<AgentConfig, OrbitError> {
        Ok(
            AgentConfig::cli(agent_cli.to_string()).with_codex_execution(
                self.context.codex_execution_policy.sandbox(),
                self.context.codex_execution_policy.approval_policy(),
            ),
        )
    }

    fn execution_environment_mode(&self, env_extra: &[String]) -> EnvironmentMode {
        if self.context.execution_env_policy.inherit() {
            EnvironmentMode::Inherit
        } else {
            EnvironmentMode::ClearAndSet(
                self.context
                    .execution_env_policy
                    .hydrated_allowlist_env_with_extras(env_extra),
            )
        }
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.context
            .execution_env_policy
            .missing_required(required_env_vars)
    }

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

    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let _ = self.update_task(
            task_id,
            TaskUpdateParams {
                title: None,
                description: None,
                plan: None,
                execution_summary: None,
                comment: None,
                status: update.status,
                branch: update.branch.map(Some),
                pr_number: update.pr_number.map(Some),
            },
        )?;
        Ok(())
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
}

fn activity_envelope_json(activity: &Activity) -> Value {
    let mut envelope = json!({
        "id": activity.id,
        "type": activity.spec_type,
        "description": activity.description,
        "input_schema_json": activity.input_schema_json,
        "output_schema_json": activity.output_schema_json,
        "identity_id": activity.identity_id,
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
