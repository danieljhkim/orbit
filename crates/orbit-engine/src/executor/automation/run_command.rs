use std::collections::HashMap;

use orbit_common::types::OrbitError;
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::tempdir;

use super::StateExecutionContext;
use crate::context::{EnvironmentHost, RuntimeHost, TaskHost};
use crate::template::{TemplateContext, render};

#[derive(Debug, Deserialize)]
struct RunCommandInput {
    command: String,
    working_dir: Option<String>,
    #[serde(default = "default_exit_codes")]
    expected_exit_codes: Vec<i32>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    env: HashMap<String, String>,
    workspace_path: Option<String>,
}

fn default_exit_codes() -> Vec<i32> {
    vec![0]
}

fn default_timeout() -> u64 {
    120
}

pub(super) fn run_command<H: RuntimeHost + TaskHost + EnvironmentHost + ?Sized>(
    host: &H,
    input: &Value,
    steps_outputs: &HashMap<String, Value>,
    state_context: Option<&StateExecutionContext>,
) -> Result<Value, OrbitError> {
    let spec: RunCommandInput = serde_json::from_value(input.clone())
        .map_err(|e| OrbitError::InvalidInput(format!("invalid run_command input: {e}")))?;

    // Build template context
    let env_pairs = host.cli_command_environment(&[]);
    let mut env: HashMap<String, String> = env_pairs.into_iter().collect();
    for (key, value) in &spec.env {
        env.insert(key.clone(), value.clone());
    }

    let template_ctx = TemplateContext {
        input: input.clone(),
        env,
        workspace_path: spec.workspace_path.clone(),
        item: None,
        iteration: None,
        steps: steps_outputs.clone(),
    };

    let command = render(&spec.command, &template_ctx)?;
    let working_dir = spec
        .working_dir
        .as_deref()
        .map(|v| render(v, &template_ctx))
        .transpose()?;

    // Set up temp dir for input file
    let temp_dir = tempdir().map_err(|e| {
        OrbitError::Execution(format!("failed to create run_command temp dir: {e}"))
    })?;
    let input_path = temp_dir.path().join("orbit-input.json");

    // Write input JSON to temp file
    let input_json = serde_json::to_string_pretty(input)
        .map_err(|e| OrbitError::Execution(format!("failed to serialize input JSON: {e}")))?;
    std::fs::write(&input_path, input_json)
        .map_err(|e| OrbitError::Execution(format!("failed to write ORBIT_INPUT_FILE: {e}")))?;

    // Build final env for the process
    let mut proc_env = template_ctx.env.clone();
    proc_env.insert(
        "ORBIT_INPUT_FILE".to_string(),
        input_path.to_string_lossy().into_owned(),
    );
    if let Some(state_context) = state_context
        && let (Some(run_id), Some(step_index), Some(state_dir)) = (
            state_context.run_id.as_ref(),
            state_context.step_index,
            state_context.state_dir.as_ref(),
        )
    {
        proc_env.insert("ORBIT_RUN_ID".to_string(), run_id.clone());
        proc_env.insert("ORBIT_MANAGED_RUN_CONTEXT".to_string(), "1".to_string());
        proc_env.insert("ORBIT_STEP_INDEX".to_string(), step_index.to_string());
        proc_env.insert(
            "ORBIT_STATE_DIR".to_string(),
            state_dir.to_string_lossy().into_owned(),
        );
    }

    let exec_result = run_process(
        &ExecRequest {
            // `command` is documented as a shell command string, so execute it
            // through a shell instead of treating the full string as argv[0].
            program: "sh".to_string(),
            args: vec!["-lc".to_string(), command],
            current_dir: working_dir,
            timeout_ms: Some(spec.timeout_seconds.saturating_mul(1000)),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::ClearAndSet(proc_env.into_iter().collect()),
            debug: false,
        },
        &NoSandbox,
    )?;

    let exit_code = exec_result.exit_code.unwrap_or_default();
    if !spec.expected_exit_codes.contains(&exit_code) {
        return Err(OrbitError::Execution(format!(
            "run_command exited with code {exit_code}; expected one of {:?}; stderr: {}",
            spec.expected_exit_codes,
            exec_result.stderr.trim()
        )));
    }

    Ok(json!({ "exit_code": exit_code }))
}
