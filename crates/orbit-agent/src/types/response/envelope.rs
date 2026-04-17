use orbit_types::{
    AgentResponseEnvelope, AgentRunError, ExecutionResult, InvocationTrace, OrbitError,
};
use serde_json::{Deserializer, Value};

use super::{AgentResponseStatus, ResponseParseResult, trace::extract_invocation_trace};

pub fn parse_and_validate_response(exec_result: &ExecutionResult) -> ResponseParseResult {
    match parse_json_envelope(exec_result) {
        Ok(parsed) => Ok(parsed),
        Err(_) => Ok(synthesize_response(exec_result)),
    }
}

pub fn is_timeout(exec_result: &ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

fn parse_json_documents(stdout: &str) -> Result<Vec<Value>, OrbitError> {
    let mut documents = Vec::new();
    for item in Deserializer::from_str(stdout).into_iter::<Value>() {
        let value = item.map_err(|error| {
            OrbitError::AgentProtocolViolation(format!("stdout is not valid JSON: {error}"))
        })?;
        documents.push(value);
    }
    if documents.is_empty() {
        return Err(OrbitError::AgentProtocolViolation(
            "stdout does not contain a JSON document".to_string(),
        ));
    }
    Ok(documents)
}

fn validate_exit_alignment(
    exec_result: &ExecutionResult,
    envelope: &AgentResponseEnvelope,
) -> Result<(), OrbitError> {
    let timed_out = is_timeout(exec_result);

    if timed_out && envelope.status != "timeout" {
        return Err(OrbitError::AgentProtocolViolation(
            "timeout process must report status=timeout".to_string(),
        ));
    }

    if timed_out {
        return Ok(());
    }

    let exit_code = exec_result.exit_code.unwrap_or(1);
    if exit_code == 0 && envelope.status != "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "exit_code=0 must report status=success".to_string(),
        ));
    }
    if exit_code != 0 && envelope.status == "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "non-zero exit code cannot report status=success".to_string(),
        ));
    }

    Ok(())
}

fn parse_json_envelope(exec_result: &ExecutionResult) -> ResponseParseResult {
    let documents = parse_json_documents(&exec_result.stdout)?;
    let envelope = documents
        .iter()
        .rev()
        .find_map(find_agent_response_envelope)
        .ok_or_else(|| {
            OrbitError::AgentProtocolViolation(
                "stdout does not contain an Orbit response envelope".to_string(),
            )
        })?;
    let trace = extract_invocation_trace(&documents, exec_result.duration_ms);

    if envelope.schema_version != 1 {
        return Err(OrbitError::AgentProtocolViolation(format!(
            "unsupported schemaVersion: {}",
            envelope.schema_version
        )));
    }

    let state = match envelope.status.as_str() {
        "success" => AgentResponseStatus::Success,
        "failed" => {
            let Some(error) = &envelope.error else {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires error object".to_string(),
                ));
            };
            if error.code.trim().is_empty() {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires non-empty error.code".to_string(),
                ));
            }
            AgentResponseStatus::Failed
        }
        "timeout" => AgentResponseStatus::Timeout,
        other => {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "unknown status: {other}"
            )));
        }
    };

    validate_exit_alignment(exec_result, &envelope)?;
    Ok((envelope, state, trace))
}

fn synthesize_response(
    exec_result: &ExecutionResult,
) -> (AgentResponseEnvelope, AgentResponseStatus, InvocationTrace) {
    if is_timeout(exec_result) {
        return (
            AgentResponseEnvelope {
                schema_version: 1,
                status: "timeout".to_string(),
                result: None,
                error: Some(AgentRunError {
                    code: "AGENT_TIMEOUT".to_string(),
                    message: "agent timed out".to_string(),
                    details: Value::Null,
                }),
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Timeout,
            InvocationTrace {
                duration_ms: exec_result.duration_ms,
                ..InvocationTrace::default()
            },
        );
    }

    if exec_result.exit_code.unwrap_or(1) == 0 {
        return (
            AgentResponseEnvelope {
                schema_version: 1,
                status: "success".to_string(),
                result: None,
                error: None,
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Success,
            InvocationTrace {
                duration_ms: exec_result.duration_ms,
                ..InvocationTrace::default()
            },
        );
    }

    (
        AgentResponseEnvelope {
            schema_version: 1,
            status: "failed".to_string(),
            result: None,
            error: Some(AgentRunError {
                code: "AGENT_INVOCATION_FAILED".to_string(),
                message: synthetic_error_message(exec_result),
                details: Value::Null,
            }),
            duration_ms: Some(exec_result.duration_ms),
        },
        AgentResponseStatus::Failed,
        InvocationTrace {
            duration_ms: exec_result.duration_ms,
            ..InvocationTrace::default()
        },
    )
}

fn synthetic_error_message(exec_result: &ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = exec_result.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    "agent execution failed".to_string()
}

fn find_agent_response_envelope(value: &Value) -> Option<AgentResponseEnvelope> {
    if let Some(envelope) = deserialize_envelope(value) {
        return Some(envelope);
    }

    match value {
        Value::String(raw) => {
            let nested = serde_json::from_str::<Value>(raw).ok()?;
            find_agent_response_envelope(&nested)
        }
        Value::Array(items) => items.iter().rev().find_map(find_agent_response_envelope),
        Value::Object(map) => {
            for key in [
                "result",
                "response",
                "message",
                "messages",
                "content",
                "final",
                "final_message",
                "output",
            ] {
                if let Some(found) = map.get(key).and_then(find_agent_response_envelope) {
                    return Some(found);
                }
            }

            map.values().find_map(find_agent_response_envelope)
        }
        _ => None,
    }
}

fn deserialize_envelope(value: &Value) -> Option<AgentResponseEnvelope> {
    let object = value.as_object()?;
    if !object.contains_key("schemaVersion") || !object.contains_key("status") {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}
