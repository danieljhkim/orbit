use std::collections::HashMap;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::JobRunState;
use orbit_types::OrbitError;
use serde::Deserialize;
use serde_json::{Value, json};

use serde_json::Value as JsonValue;

use super::ActivityExecutor;
use crate::activity_runner::{
    execution_template_context_with_env, validate_activity_output_schema,
};
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AttemptOutcome, EnvironmentHost, ExecutionContext, ExecutorHost,
    TaskReadHost, state_env_vars,
};
use crate::template::{TemplateContext, render};
use orbit_types::InvocationTrace;

#[derive(Debug, Clone, Deserialize)]
pub struct CliCommandSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub timeout_seconds: Option<u64>,
    #[serde(default = "default_exit_codes")]
    pub expected_exit_codes: Vec<i32>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_exit_codes() -> Vec<i32> {
    vec![0]
}

pub struct CliCommandExecutor;

impl ActivityExecutor for CliCommandExecutor {
    fn spec_type(&self) -> &str {
        "cli_command"
    }

    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        let cli_host = host.cli();
        let mut cli_env = cli_host.cli_command_environment(&execution.env_extra);
        // Apply explicit env_set overrides on top of the resolved environment.
        for (key, value) in &execution.env_set {
            if let Some(existing) = cli_env.iter_mut().find(|(k, _)| k == key) {
                existing.1 = value.clone();
            } else {
                cli_env.push((key.clone(), value.clone()));
            }
        }
        let mut template_context = execution_template_context_with_env(execution, cli_env);
        // When a cli_command step has task_id in its input, load the task and
        // inject its fields into the template context so {{workspace_path}}
        // resolves from the task without explicit pipeline input.
        if template_context.workspace_path.is_none()
            && let Some(task_id) = execution.input.get("task_id").and_then(JsonValue::as_str)
            && let Ok(task) = cli_host.get_task(task_id)
        {
            template_context.workspace_path = task.workspace_path;
        }
        match execute(
            execution,
            &execution.activity.spec_config,
            &template_context,
            execution.timeout_seconds,
        ) {
            Ok((result, duration_ms, exit_code)) => {
                if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                    return AttemptOutcome {
                        exit_code,
                        duration_ms: Some(duration_ms),
                        response_json: Some(result),
                        ..AttemptOutcome::failed(ACTIVITY_EXECUTION_FAILED, err.to_string())
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code,
                    duration_ms: Some(duration_ms),
                    invocation_trace: InvocationTrace {
                        duration_ms,
                        ..InvocationTrace::default()
                    },
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                    retry_count: 0,
                }
            }
            Err(err) => AttemptOutcome::failed(ACTIVITY_EXECUTION_FAILED, err.to_string()),
        }
    }
}

pub fn execute(
    execution: &ExecutionContext,
    spec_config: &Value,
    template_context: &TemplateContext,
    timeout_seconds: u64,
) -> Result<(Value, u64, Option<i32>), OrbitError> {
    let spec: CliCommandSpec = serde_json::from_value(spec_config.clone()).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid cli_command spec_config: {error}"))
    })?;

    let command = render(&spec.command, template_context)?;
    let args = spec
        .args
        .iter()
        .map(|arg| render(arg, template_context))
        .collect::<Result<Vec<_>, OrbitError>>()?;
    let current_dir = spec
        .working_dir
        .as_deref()
        .map(|value| render(value, template_context))
        .transpose()?;

    let mut env = template_context.env.clone();
    for (key, value) in &spec.env {
        env.insert(key.clone(), render(value, template_context)?);
    }
    for (key, value) in state_env_vars(execution) {
        env.insert(key, value);
    }

    let exec_result = run_process(
        &ExecRequest {
            program: command,
            args,
            current_dir,
            timeout_ms: Some(
                spec.timeout_seconds
                    .unwrap_or(timeout_seconds)
                    .saturating_mul(1000),
            ),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::ClearAndSet(env.into_iter().collect()),
            debug: false,
        },
        &NoSandbox,
    )?;

    let exit_code = exec_result.exit_code.unwrap_or_default();
    if !spec.expected_exit_codes.contains(&exit_code) {
        return Err(OrbitError::Execution(format!(
            "cli_command exited with code {exit_code}; expected one of {:?}; stderr: {}",
            spec.expected_exit_codes,
            exec_result.stderr.trim()
        )));
    }

    Ok((
        json!({ "exit_code": exit_code }),
        exec_result.duration_ms,
        exec_result.exit_code,
    ))
}
