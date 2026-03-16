use std::io::Write;

use orbit_agent::{
    Agent, AgentConfig, AgentRequest, AgentResponseStatus, parse_and_validate_response,
};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{AgentCommitRequest, AgentResponseEnvelope, JobRunState, OrbitError};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use crate::OrbitRuntime;
use crate::command::activity::activity_skill_refs_from_spec_config;
use crate::engine::{
    AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED, AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT,
    AttemptOutcome, ExecutionContext, execution_working_directory,
};
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

pub(crate) fn execute(runtime: &OrbitRuntime, execution: &ExecutionContext) -> AttemptOutcome {
    let invocation = match build_agent_invocation(runtime, execution) {
        Ok(invocation) => invocation,
        Err(outcome) => return outcome,
    };
    let exec_result = match execute_agent_process(runtime, execution, invocation) {
        Ok(result) => result,
        Err(outcome) => return outcome,
    };

    if orbit_agent::is_timeout(&exec_result) && exec_result.stdout.trim().is_empty() {
        return AttemptOutcome {
            state: JobRunState::Timeout,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_TIMEOUT.to_string()),
            error_message: Some(format_timeout_error_message(&exec_result)),
            protocol_violation: false,
        };
    }

    match parse_and_validate_response(&exec_result) {
        Ok((envelope, state)) => {
            process_agent_response(runtime, execution, &exec_result, envelope, state)
        }
        Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
            error_message: Some(message),
            protocol_violation: true,
        },
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
        },
    }
}

fn build_agent_invocation(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
) -> Result<orbit_agent::AgentResponse, AttemptOutcome> {
    let agent = Agent::new(
        &AgentConfig::cli(execution.agent_cli.clone()).with_codex_execution(
            runtime.context.codex_execution_policy.sandbox(),
            runtime.context.codex_execution_policy.approval_policy(),
        ),
    )
    .map_err(invocation_failed_outcome)?;
    let stdin_payload = runtime
        .build_stdin_envelope_payload(execution)
        .map_err(invocation_failed_outcome)?;

    let invocation = match &execution.job {
        Some(job) => agent.invoke(AgentRequest::job(
            job.job_id.clone(),
            execution.activity.id.clone(),
            stdin_payload,
        )),
        None => agent.invoke(AgentRequest::activity(
            execution.activity.id.clone(),
            stdin_payload,
        )),
    }
    .map_err(invocation_failed_outcome)?;

    let missing_env = runtime
        .context
        .execution_env_policy
        .missing_required(invocation.required_env_vars);
    if !missing_env.is_empty() {
        let vars = missing_env.join(", ");
        return Err(AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
            error_message: Some(format!(
                "missing required environment variable(s) for provider '{}': {vars}. \
configure .orbit/config.toml [execution.env].pass and set these variables in the parent shell.",
                invocation.runtime_key
            )),
            protocol_violation: false,
        });
    }

    Ok(invocation)
}

fn execute_agent_process(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
    invocation: orbit_agent::AgentResponse,
) -> Result<orbit_types::ExecutionResult, AttemptOutcome> {
    let environment_mode = if runtime.context.execution_env_policy.inherit() {
        EnvironmentMode::Inherit
    } else {
        EnvironmentMode::ClearAndSet(
            runtime
                .context
                .execution_env_policy
                .hydrated_allowlist_env_with_extras(&execution.env_extra),
        )
    };
    let (args, _stdout_schema_file) =
        prepare_exec_args(&invocation).map_err(invocation_failed_outcome)?;

    run_process(
        &ExecRequest {
            program: invocation.program,
            args,
            current_dir: execution_working_directory(execution),
            timeout_ms: Some(execution.timeout_seconds.saturating_mul(1000)),
            stdin_mode: StdinMode::Bytes(invocation.stdin),
            environment_mode,
        },
        &NoSandbox,
    )
    .map_err(invocation_failed_outcome)
}

fn process_agent_response(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: AgentResponseEnvelope,
    state: AgentResponseStatus,
) -> AttemptOutcome {
    let run_state = match state {
        AgentResponseStatus::Success => JobRunState::Success,
        AgentResponseStatus::Failed => JobRunState::Failed,
        AgentResponseStatus::Timeout => JobRunState::Timeout,
    };
    let error_code = envelope.error.as_ref().map(|error| error.code.clone());
    let error_message = envelope.error.as_ref().map(|error| error.message.clone());

    if let Some(outcome) =
        validate_agent_success(runtime, execution, exec_result, &envelope, run_state)
    {
        return outcome;
    }

    AttemptOutcome {
        state: run_state,
        exit_code: exec_result.exit_code,
        duration_ms: Some(exec_result.duration_ms),
        response_json: serde_json::to_value(envelope).ok(),
        error_code,
        error_message,
        protocol_violation: false,
    }
}

fn validate_agent_success(
    runtime: &OrbitRuntime,
    execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: &AgentResponseEnvelope,
    run_state: JobRunState,
) -> Option<AttemptOutcome> {
    if run_state == JobRunState::Success
        && envelope.result.is_some()
        && let Err(err) = runtime.validate_skill_output_schema(&execution.activity, envelope)
    {
        return Some(AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: true,
        });
    }
    if run_state == JobRunState::Success
        && let Some(result) = envelope.result.as_ref()
        && let Err(err) = runtime.execute_commit_request_if_present(result)
    {
        let (error_code, protocol_violation) = match err {
            OrbitError::AgentProtocolViolation(_) => (AGENT_PROTOCOL_VIOLATION.to_string(), true),
            _ => (AGENT_COMMIT_FAILED.to_string(), false),
        };
        return Some(AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: serde_json::to_value(envelope).ok(),
            error_code: Some(error_code),
            error_message: Some(err.to_string()),
            protocol_violation,
        });
    }

    None
}

fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    AttemptOutcome {
        state: JobRunState::Failed,
        exit_code: Some(1),
        duration_ms: None,
        response_json: None,
        error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
        error_message: Some(err.to_string()),
        protocol_violation: false,
    }
}

impl OrbitRuntime {
    fn build_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        let skill_refs = activity_skill_refs_from_spec_config(&execution.activity.spec_config)?;
        let skills = self.resolve_activity_skill_refs(&skill_refs)?;
        let identity = execution
            .activity
            .identity_id
            .as_deref()
            .map(|identity_id| self.resolve_identity(identity_id))
            .transpose()?
            .map(|resolved| {
                json!({
                    "id": resolved.id,
                    "name": resolved.name,
                    "role": resolved.role.to_string(),
                    "block": self.compile_identity_block(&resolved),
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
        &self,
        activity: &orbit_types::Activity,
        envelope: &AgentResponseEnvelope,
    ) -> Result<(), OrbitError> {
        let skill_refs = activity_skill_refs_from_spec_config(&activity.spec_config)?;
        let skills = self.resolve_activity_skill_refs(&skill_refs)?;
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

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
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

        let repo_root = paths::find_git_repo_root(&self.context.data_root).ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot locate git repository root from Orbit data root '{}'",
                self.context.data_root.display()
            ))
        })?;
        let repo_root_str = repo_root.to_string_lossy().to_string();

        self.run_tool(
            "git.stage_paths",
            json!({
                "repo_root": repo_root_str,
                "files": files.clone(),
            }),
        )?;
        self.run_tool(
            "git.commit",
            json!({
                "repo_root": repo_root.to_string_lossy(),
                "message": message,
                "files": files,
            }),
        )?;
        Ok(())
    }
}

fn prepare_exec_args(
    invocation: &orbit_agent::AgentResponse,
) -> Result<(Vec<String>, Option<NamedTempFile>), OrbitError> {
    let mut args = invocation.args.clone();
    let mut stdout_schema_file = None;

    if let Some(schema) = invocation.stdout_schema_json.as_ref() {
        let mut file = NamedTempFile::new().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create temporary agent output schema file: {error}"
            ))
        })?;
        serde_json::to_writer(file.as_file_mut(), schema).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to write temporary agent output schema file: {error}"
            ))
        })?;
        file.as_file_mut().flush().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to flush temporary agent output schema file: {error}"
            ))
        })?;

        args.push("--output-schema".to_string());
        args.push(file.path().to_string_lossy().into_owned());
        stdout_schema_file = Some(file);
    }

    Ok((args, stdout_schema_file))
}

fn format_timeout_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if stderr.is_empty() {
        return "agent timed out before producing JSON stdout".to_string();
    }
    format!("agent timed out before producing JSON stdout; stderr: {stderr}")
}

fn activity_envelope_json(activity: &orbit_types::Activity) -> Value {
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
