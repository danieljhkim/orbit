use orbit_core::job::agent_protocol::{
    StdinAdapter, build_invocation, build_stdin_payload, parse_and_validate_response,
};
use orbit_types::{ExecutionResult, JobRunState, JobTargetType, OrbitError};
use serde_json::json;

fn expected_native_args() -> Vec<String> {
    vec![
        "run".to_string(),
        "--target-type".to_string(),
        "work".to_string(),
        "--target-id".to_string(),
        "spec-123".to_string(),
        "--mode".to_string(),
        "scheduled".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]
}

#[test]
fn provider_mapper_supports_claude() {
    let invocation =
        build_invocation("claude", JobTargetType::Work, "spec-123").expect("claude mapper");
    assert_eq!(invocation.program, "claude");
    assert_eq!(
        invocation.args,
        vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "text".to_string()
        ]
    );
    assert_eq!(
        invocation.stdin_adapter,
        StdinAdapter::PromptWithEmbeddedEnvelope
    );
}

#[test]
fn provider_mapper_supports_codex() {
    let invocation =
        build_invocation("codex", JobTargetType::Work, "spec-123").expect("codex mapper");
    assert_eq!(invocation.program, "codex");
    assert_eq!(invocation.args, vec!["exec".to_string()]);
    assert_eq!(
        invocation.stdin_adapter,
        StdinAdapter::PromptWithEmbeddedEnvelope
    );
}

#[test]
fn provider_mapper_supports_mock_agent() {
    let invocation =
        build_invocation("mock-agent", JobTargetType::Work, "spec-123").expect("mock-agent mapper");
    assert_eq!(invocation.program, "mock-agent");
    assert_eq!(invocation.args, expected_native_args());
    assert_eq!(invocation.stdin_adapter, StdinAdapter::OrbitEnvelopeJson);
}

#[test]
fn provider_mapper_uses_binary_basename_for_paths() {
    let invocation = build_invocation("/usr/local/bin/claude", JobTargetType::Work, "spec-123")
        .expect("path-based claude mapper");
    assert_eq!(invocation.program, "/usr/local/bin/claude");
    assert_eq!(
        invocation.args,
        vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "text".to_string()
        ]
    );
    assert_eq!(
        invocation.stdin_adapter,
        StdinAdapter::PromptWithEmbeddedEnvelope
    );
}

#[test]
fn provider_mapper_rejects_unsupported_provider() {
    let err = build_invocation("custom-agent", JobTargetType::Work, "spec-123")
        .expect_err("unsupported provider must fail");
    assert!(matches!(
        err,
        OrbitError::UnsupportedAgentProvider(provider) if provider == "custom-agent"
    ));
}

#[test]
fn protocol_parser_accepts_success_envelope() {
    let exec = ExecutionResult {
        success: true,
        stdout: serde_json::to_string(&json!({
            "schemaVersion": 1,
            "status": "success",
            "result": {"ok": true},
            "error": null,
            "durationMs": 55
        }))
        .expect("serialize"),
        stderr: String::new(),
        exit_code: Some(0),
        duration_ms: 55,
        output: None,
    };

    let (envelope, state) = parse_and_validate_response(&exec).expect("valid envelope");
    assert_eq!(state, JobRunState::Success);
    assert_eq!(envelope.status, "success");
}

#[test]
fn stdin_payload_wraps_envelope_for_prompt_based_providers() {
    let invocation =
        build_invocation("codex", JobTargetType::Work, "spec-123").expect("codex mapper");
    let payload = build_stdin_payload(&invocation, br#"{"schemaVersion":1}"#);
    let text = String::from_utf8(payload).expect("utf8");
    assert!(text.contains("Execution envelope"));
    assert!(text.contains(r#"{"schemaVersion":1}"#));
}

#[test]
fn protocol_parser_surfaces_stderr_when_stdout_empty() {
    let exec = ExecutionResult {
        success: false,
        stdout: String::new(),
        stderr: "fatal: permission denied".to_string(),
        exit_code: Some(1),
        duration_ms: 1,
        output: None,
    };

    let err = parse_and_validate_response(&exec).expect_err("must fail");
    let msg = err.to_string();
    assert!(msg.contains("agent stdout is empty"));
    assert!(msg.contains("permission denied"));
}
