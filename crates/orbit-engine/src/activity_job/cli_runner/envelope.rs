use orbit_agent::parse_and_validate_response;
use orbit_common::types::activity_job::AgentLoopSpec;
use orbit_common::types::{ExecutionResult, InvocationTrace};
use serde_json::Value;

use super::super::dispatcher::DispatchError;

pub(super) fn cli_agent_envelope_json(
    spec: &AgentLoopSpec,
    run_id: &str,
    input: &Value,
    task_ctx: Option<&Value>,
    prompt_override: Option<&str>,
) -> Result<Vec<u8>, DispatchError> {
    let mut envelope = serde_json::Map::new();
    envelope.insert("schemaVersion".to_string(), Value::from(1));
    envelope.insert(
        "instruction".to_string(),
        Value::String(spec.instruction.clone()),
    );
    envelope.insert(
        "prompt".to_string(),
        Value::String(match prompt_override {
            Some(prompt) => prompt.to_string(),
            None => user_prompt_from_input(input)?,
        }),
    );
    envelope.insert("input".to_string(), input.clone());
    envelope.insert("run_id".to_string(), Value::String(run_id.to_string()));
    envelope.insert(
        "tools".to_string(),
        serde_json::to_value(&spec.tools)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize tools: {err}")))?,
    );
    envelope.insert(
        "model".to_string(),
        serde_json::to_value(&spec.model)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize model: {err}")))?,
    );

    if let Some(task) = task_ctx {
        envelope.insert("task".to_string(), task.clone());
    }

    serde_json::to_vec(&Value::Object(envelope))
        .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize envelope: {err}")))
}

pub(super) fn parse_cli_invocation_trace(
    stdout: &[u8],
    stderr: &[u8],
    exit_code: Option<i32>,
    duration_ms: u64,
    success: bool,
) -> Option<InvocationTrace> {
    let exec_result = ExecutionResult {
        success,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        exit_code,
        duration_ms,
        output: None,
    };

    parse_and_validate_response(&exec_result)
        .map(|(_, _, trace)| trace)
        .ok()
}

pub(super) fn user_prompt_from_input(input: &Value) -> Result<String, DispatchError> {
    match input {
        Value::Object(map) => match map.get("prompt") {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(other) => serde_json::to_string(other).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
            None => serde_json::to_string(input).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
        },
        Value::String(text) => Ok(text.clone()),
        Value::Null => Ok(String::new()),
        other => serde_json::to_string(other)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))),
    }
}

pub(in crate::activity_job) fn task_id_from_input(input: &Value) -> Option<&str> {
    fn non_empty(value: &str) -> Option<&str> {
        if value.is_empty() { None } else { Some(value) }
    }

    input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            input
                .get("task")
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .or_else(|| {
            input
                .get("task_ids")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find_map(Value::as_str))
                .and_then(non_empty)
        })
}

#[cfg(test)]
mod tests;
