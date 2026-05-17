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

fn synthesize_response(
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
fn synthesize_trace(exec_result: &ExecutionResult) -> InvocationTrace {
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

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::ExecutionResult;

    fn exec(stdout: &str, stderr: &str, exit_code: Option<i32>, success: bool) -> ExecutionResult {
        ExecutionResult {
            success,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
            duration_ms: 1234,
            output: None,
        }
    }

    #[test]
    fn synthesize_trace_preserves_usage_from_provider_json_without_envelope() {
        // Provider-shaped JSON with usage but no Orbit envelope; agent failed
        // (non-zero exit) so the synthesize fallback runs. Token totals from
        // the outer JSON must survive instead of being zeroed.
        let stdout = r#"{"type":"result","usage":{"input_tokens":42,"output_tokens":7}}"#;
        let result = parse_and_validate_response(&exec(stdout, "", Some(1), false));
        // No envelope means the synthesize fallback only succeeds if stdout is
        // empty; with content but exit!=0, parse_and_validate returns Err. We
        // exercise synthesize_trace directly to verify the trace contents the
        // synthesize path WOULD return.
        assert!(result.is_err(), "expected envelope parse to fail");

        let trace = synthesize_trace(&exec(stdout, "", Some(1), false));
        assert_eq!(trace.usage.input, 42);
        assert_eq!(trace.usage.output, 7);
        assert_eq!(trace.duration_ms, 1234);
    }

    #[test]
    fn synthesize_trace_preserves_claude_outer_usage_when_envelope_invalid() {
        // Mimics `claude -p --output-format json` output: outer `usage` plus
        // a `result` string that does NOT contain a valid Orbit envelope (e.g.
        // claude failed mid-flight and emitted free text). Outer usage must
        // still be captured.
        let stdout = r#"{"type":"result","subtype":"success","result":"plain text reply, not an envelope","usage":{"input_tokens":1000,"output_tokens":250,"cache_read_input_tokens":500,"cache_creation_input_tokens":100}}"#;
        let trace = synthesize_trace(&exec(stdout, "", Some(0), true));
        assert_eq!(trace.usage.input, 1000);
        assert_eq!(trace.usage.output, 250);
        assert_eq!(trace.usage.cache_read, 500);
        assert_eq!(trace.usage.cache_create, 100);
    }

    #[test]
    fn synthesize_trace_falls_back_to_duration_only_when_stdout_unparseable() {
        // Plain non-JSON stdout: regression check that the previous "duration
        // only, zero usage" behavior is preserved when documents can't be
        // parsed at all.
        let trace = synthesize_trace(&exec("agent crashed", "stderr noise", Some(2), false));
        assert_eq!(trace.usage.input, 0);
        assert_eq!(trace.usage.output, 0);
        assert_eq!(trace.duration_ms, 1234);
    }

    #[test]
    fn synthesize_trace_handles_empty_stdout() {
        // Empty stdout returns a parse error from serde; synthesize_trace must
        // still return a trace with duration set.
        let trace = synthesize_trace(&exec("", "boom", Some(1), false));
        assert_eq!(trace.usage.input, 0);
        assert_eq!(trace.usage.output, 0);
        assert_eq!(trace.duration_ms, 1234);
    }

    #[test]
    fn peek_response_status_extracts_envelope_failed_from_claude_shaped_wrapper() {
        // Mimics the bug in T20260508-17: claude exits 0 with `result.subtype`
        // = "success" but the inner Orbit envelope (carried as a JSON-string
        // in `result`) reports `status: "failed"`. peek_response_status must
        // surface "failed" so the dispatcher can demote success without going
        // through validate_exit_alignment (which would reject the envelope
        // outright because exit==0 contradicts status=="failed").
        let inner = r#"{\"schemaVersion\":1,\"status\":\"failed\",\"error\":{\"code\":\"E\",\"message\":\"m\",\"details\":null}}"#;
        let stdout = format!(
            r#"{{"type":"result","subtype":"success","result":"{inner}","usage":{{"input_tokens":10,"output_tokens":3}}}}"#
        );
        assert_eq!(peek_response_status(&stdout).as_deref(), Some("failed"));
    }

    #[test]
    fn peek_response_status_extracts_failed_from_prose_prefixed_claude_result() {
        let result = concat!(
            "I could not continue after the workspace disappeared.\n",
            r#"{"schemaVersion":1,"status":"failed","error":{"code":"workspace_unavailable","message":"worktree missing","details":null}}"#
        );
        let stdout = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "result": result,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 3
            }
        })
        .to_string();

        assert_eq!(peek_response_status(&stdout).as_deref(), Some("failed"));
    }

    #[test]
    fn peek_response_status_returns_none_when_no_envelope_present() {
        assert_eq!(peek_response_status("{\"hello\":\"world\"}"), None);
        assert_eq!(peek_response_status("{\"status\":\"failed\"}"), None);
        let prose_with_braces = serde_json::json!({
            "result": "prose with {arbitrary braces} and {\"status\":\"failed\"}, but no Orbit envelope"
        })
        .to_string();
        assert_eq!(peek_response_status(&prose_with_braces), None);
        assert_eq!(peek_response_status(""), None);
        assert_eq!(peek_response_status("not json"), None);
    }

    #[test]
    fn peek_response_status_extracts_success_from_top_level_envelope() {
        let stdout = r#"{"schemaVersion":1,"status":"success","result":{}}"#;
        assert_eq!(peek_response_status(stdout).as_deref(), Some("success"));
    }

    #[test]
    fn synthesize_response_failed_path_carries_usage() {
        // Empty stdout + non-zero exit triggers the synthesize "failed" path.
        // The trace returned alongside the synthesized envelope must preserve
        // usage when stdout is parseable, but here it's empty so usage stays
        // zero — verifies the synthesized envelope is wired to synthesize_trace.
        let exec = exec("", "agent crashed", Some(1), false);
        let (envelope, status, trace) = synthesize_response(&exec).expect("synthesized");
        assert_eq!(envelope.status, "failed");
        assert_eq!(status, AgentResponseStatus::Failed);
        assert_eq!(trace.duration_ms, 1234);
        assert_eq!(trace.usage.input, 0);
    }

    #[test]
    fn grok_like_cli_response_extracts_nonzero_usage_and_tool_calls() {
        // Grok CLI --output-format json returns a wrapper with "text" containing
        // the Orbit envelope (plus any usage/tool metadata the CLI attaches).
        // The extraction must descend into "text" content to surface non-zero
        // token usage and tool invocations for diagnostics/metrics.
        let inner = r#"{"schemaVersion":1,"status":"success","result":{"pong":"grok"},"error":null,"usage":{"input_tokens":120,"output_tokens":35},"tool_calls":[{"id":"tc1","name":"fs.read"}]}"#;
        let stdout = serde_json::json!({
            "text": inner,
            "stopReason": "EndTurn"
        })
        .to_string();
        let exec = exec(&stdout, "", Some(0), true);
        let (_, _, trace) = parse_and_validate_response(&exec).expect("grok-like parses");
        assert_eq!(trace.usage.input, 120);
        assert_eq!(trace.usage.output, 35);
        assert!(!trace.tool_calls.is_empty());
        assert_eq!(trace.tool_calls[0].tool_name, "fs.read");
    }
}
