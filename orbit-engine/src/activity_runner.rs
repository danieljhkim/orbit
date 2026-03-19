use orbit_types::{Activity, Job, JobRunState, JobStep, OrbitError};
use serde_json::{Value, json};

use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AttemptOutcome, DirectActivityRunOutcome, EngineHost,
    ExecutionContext, input_workspace_path, redact_attempt_outcome,
};
use crate::executor::{agent, api, automation, cli_command};
use crate::json_schema::validate_instance_against_schema;
use crate::template::TemplateContext;

pub fn run_activity_direct<H: EngineHost>(
    host: &H,
    activity: &Activity,
    agent_cli: &str,
    timeout_seconds: u64,
) -> Result<DirectActivityRunOutcome, OrbitError> {
    let execution = ExecutionContext {
        activity: activity.clone(),
        job: None,
        agent_cli: agent_cli.to_string(),
        timeout_seconds,
        env_extra: vec![],
        input: json!({}),
    };
    let outcome = execute_single_attempt(host, &execution);
    Ok(DirectActivityRunOutcome {
        state: outcome.state,
        duration_ms: outcome.duration_ms,
        error_code: outcome.error_code,
        error_message: outcome.error_message,
        protocol_violation: outcome.protocol_violation,
    })
}

pub fn build_execution_context_for_step<H: EngineHost>(
    host: &H,
    job: &Job,
    step: &JobStep,
    input: Value,
) -> Result<ExecutionContext, OrbitError> {
    let activity = host.validate_activity_target_exists(step.target_type, &step.target_id)?;
    validate_activity_input_schema(&activity, &input)?;
    Ok(ExecutionContext {
        activity,
        job: Some(job.clone()),
        agent_cli: step.agent_cli.clone(),
        timeout_seconds: step.timeout_seconds,
        env_extra: step.env_extra.clone(),
        input,
    })
}

pub fn execute_single_attempt<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    let outcome = match execution.activity.spec_type.as_str() {
        "agent_invoke" => agent::execute(host, execution),
        "cli_command" => execute_cli_command_attempt(host, execution),
        "api" => execute_api_attempt(execution),
        "automation" => execute_automation_attempt(host, execution),
        other => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
            error_message: Some(format!("unsupported activity spec_type '{other}'")),
            protocol_violation: false,
        },
    };
    redact_attempt_outcome(outcome)
}

fn execute_cli_command_attempt<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    let template_context = execution_template_context_with_env(
        execution,
        host.cli_command_environment(&execution.env_extra),
    );
    match cli_command::execute(
        &execution.activity.spec_config,
        &template_context,
        execution.timeout_seconds,
    ) {
        Ok((result, duration_ms, exit_code)) => {
            if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code,
                    duration_ms: Some(duration_ms),
                    response_json: Some(result),
                    error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    protocol_violation: false,
                };
            }
            AttemptOutcome {
                state: JobRunState::Success,
                exit_code,
                duration_ms: Some(duration_ms),
                response_json: Some(result),
                error_code: None,
                error_message: None,
                protocol_violation: false,
            }
        }
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
        },
    }
}

fn execute_api_attempt(execution: &ExecutionContext) -> AttemptOutcome {
    let template_context = execution_template_context(execution);
    match api::execute(
        &execution.activity.spec_config,
        &template_context,
        execution.timeout_seconds,
    ) {
        Ok(result) => {
            if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    protocol_violation: false,
                };
            }
            AttemptOutcome {
                state: JobRunState::Success,
                exit_code: Some(0),
                duration_ms: None,
                response_json: Some(result),
                error_code: None,
                error_message: None,
                protocol_violation: false,
            }
        }
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
        },
    }
}

fn execute_automation_attempt<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    match automation::execute(host, &execution.activity, &execution.input) {
        Ok(result) => {
            if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                    error_message: Some(err.to_string()),
                    protocol_violation: false,
                };
            }
            AttemptOutcome {
                state: JobRunState::Success,
                exit_code: Some(0),
                duration_ms: None,
                response_json: Some(result),
                error_code: None,
                error_message: None,
                protocol_violation: false,
            }
        }
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(1),
            duration_ms: None,
            response_json: None,
            error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
        },
    }
}

pub fn execution_template_context(execution: &ExecutionContext) -> TemplateContext {
    execution_template_context_with_env(execution, std::env::vars().collect())
}

fn execution_template_context_with_env(
    execution: &ExecutionContext,
    env_pairs: Vec<(String, String)>,
) -> TemplateContext {
    let mut env = env_pairs
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    env.insert("ORBIT_TASK_ACTOR_KIND".to_string(), "agent".to_string());
    if let Some(identity_id) = execution.activity.identity_id.as_ref() {
        env.insert(
            "ORBIT_TASK_ACTOR_IDENTITY_ID".to_string(),
            identity_id.clone(),
        );
    }

    TemplateContext {
        input: execution.input.clone(),
        env,
        workspace_path: execution
            .activity
            .workspace_path
            .clone()
            .or_else(|| input_workspace_path(&execution.input)),
    }
}

pub fn validate_activity_input_schema(
    activity: &Activity,
    input: &Value,
) -> Result<(), OrbitError> {
    let context = format!(
        "job run input does not match activity '{}' input schema",
        activity.id
    );
    match validate_instance_against_schema(&activity.input_schema_json, input, &context) {
        Ok(()) => Ok(()),
        Err(OrbitError::AgentProtocolViolation(message)) => Err(OrbitError::InvalidInput(message)),
        Err(other) => Err(other),
    }
}

pub fn validate_activity_output_schema(
    activity: &Activity,
    output: &Value,
) -> Result<(), OrbitError> {
    let context = format!(
        "activity '{}' output does not match output schema",
        activity.id
    );
    validate_instance_against_schema(&activity.output_schema_json, output, &context)
}

pub fn activity_skill_refs_from_spec_config(
    spec_config: &Value,
) -> Result<Vec<String>, OrbitError> {
    ensure_spec_config_object(spec_config)?;
    let Some(raw_refs) = spec_config.get("skill_refs") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(raw_refs.clone()).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "activity spec_config.skill_refs must be an array of strings: {error}"
        ))
    })
}

fn ensure_spec_config_object(spec_config: &Value) -> Result<(), OrbitError> {
    if spec_config.is_object() {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput(
            "activity spec_config must be a JSON object".to_string(),
        ))
    }
}
