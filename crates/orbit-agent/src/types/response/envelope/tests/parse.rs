#![allow(missing_docs)]

use orbit_common::types::ExecutionResult;

use super::super::*;

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
