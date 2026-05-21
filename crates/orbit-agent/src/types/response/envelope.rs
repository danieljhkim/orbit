use orbit_common::types::{
    AgentResponseEnvelope, AgentRunError, ExecutionResult, InvocationTrace, OrbitError,
};
use serde::Deserialize;
use serde_json::{Deserializer, Value};

use super::{AgentResponseStatus, ResponseParseResult, trace::extract_invocation_trace};

pub fn parse_and_validate_response(exec_result: &ExecutionResult) -> ResponseParseResult {
    match parse_json_envelope(exec_result) {
        Ok(parsed) => Ok(parsed),
        Err(err) => synthesize_response(exec_result).ok_or(err),
    }
}

pub fn is_timeout(exec_result: &ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

/// Best-effort lookup of an embedded Orbit response envelope's `status` field
/// in raw subprocess stdout, *without* validating exit-code alignment.
///
/// Used by the CLI dispatcher (T20260508-17) to demote `success` when a CLI
/// like Claude exits 0 with a wrapping `result.subtype = "success"` but its
/// embedded Orbit envelope reports `status = "failed"`. `parse_and_validate_response`
/// returns `Err` in that case because exit alignment fails, which threw away
/// the signal the dispatcher needs to classify the outcome.
///
/// Returns `None` when stdout cannot be parsed or carries no recognizable
/// envelope, so callers can fall through to other classification rather than
/// regressing legacy provider shapes.
pub fn peek_response_status(stdout: &str) -> Option<String> {
    let documents = parse_json_documents(stdout).ok()?;
    let envelope = documents
        .iter()
        .rev()
        .find_map(find_agent_response_envelope)?;
    Some(envelope.status)
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

// Visible through `response.rs` to sibling-layout tests; keeping this private
// would require nesting tests back under `envelope`.
pub(in crate::types) fn synthesize_response(
    exec_result: &ExecutionResult,
) -> Option<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace)> {
    if is_timeout(exec_result) {
        return Some((
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
            synthesize_trace(exec_result),
        ));
    }

    if exec_result.exit_code.unwrap_or(1) == 0 || !exec_result.stdout.trim().is_empty() {
        return None;
    }

    Some((
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
        synthesize_trace(exec_result),
    ))
}

// Best-effort trace extraction for the fallback path. Provider CLIs (e.g.
// `claude -p --output-format json`) emit a wrapping JSON document whose
// `usage` block carries token counts even when the embedded Orbit response
// envelope is malformed or missing — losing that data on the synthesize path
// is what made claude show as zero tokens on the scoreboard.
// Visible through `response.rs` to sibling-layout tests; this is a narrow
// crate-internal seam for fallback trace behavior.
pub(in crate::types) fn synthesize_trace(exec_result: &ExecutionResult) -> InvocationTrace {
    match parse_json_documents(&exec_result.stdout) {
        Ok(documents) => extract_invocation_trace(&documents, exec_result.duration_ms),
        Err(_) => InvocationTrace {
            duration_ms: exec_result.duration_ms,
            ..InvocationTrace::default()
        },
    }
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
        Value::String(raw) => find_agent_response_envelope_in_string(raw),
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

fn find_agent_response_envelope_in_string(raw: &str) -> Option<AgentResponseEnvelope> {
    if let Ok(nested) = serde_json::from_str::<Value>(raw)
        && let Some(envelope) = find_agent_response_envelope(&nested)
    {
        return Some(envelope);
    }

    raw.match_indices('{').find_map(|(start, _)| {
        let mut deserializer = Deserializer::from_str(&raw[start..]);
        let nested = Value::deserialize(&mut deserializer).ok()?;
        find_agent_response_envelope(&nested)
    })
}

fn deserialize_envelope(value: &Value) -> Option<AgentResponseEnvelope> {
    let object = value.as_object()?;
    if !object.contains_key("schemaVersion") || !object.contains_key("status") {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}
