use orbit_agent::{
    Agent, AgentConfig, AgentRequest, AgentResponseStatus, ProviderOptions,
    parse_and_validate_response,
};
use orbit_types::{ExecutionResult, OrbitError};
use serde_json::json;

fn activity_request() -> AgentRequest {
    AgentRequest::activity("spec-123", br#"{"schemaVersion":1}"#.to_vec())
}

fn job_request() -> AgentRequest {
    AgentRequest::job("job-123", "spec-123", br#"{"schemaVersion":1}"#.to_vec())
}

fn expected_job_args() -> Vec<String> {
    vec![
        "run".to_string(),
        "--target-type".to_string(),
        "activity".to_string(),
        "--target-id".to_string(),
        "spec-123".to_string(),
        "--job-id".to_string(),
        "job-123".to_string(),
        "--mode".to_string(),
        "job".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]
}

fn expected_activity_args() -> Vec<String> {
    vec![
        "run".to_string(),
        "--target-type".to_string(),
        "activity".to_string(),
        "--target-id".to_string(),
        "spec-123".to_string(),
        "--mode".to_string(),
        "activity".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]
}

#[test]
fn provider_mapper_supports_claude() {
    let agent =
        Agent::new(&AgentConfig::cli("claude").expect("claude config")).expect("claude runtime");
    let invocation = agent.invoke(job_request()).expect("claude invocation");
    assert_eq!(invocation.program, "claude");
    assert_eq!(
        invocation.args,
        vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
            "--no-session-persistence".to_string(),
        ]
    );
    assert_eq!(invocation.runtime_key, "claude");
    let text = String::from_utf8(invocation.stdin).expect("utf8");
    assert!(text.contains("Execution envelope"));
}

#[test]
fn provider_mapper_supports_codex() {
    let agent =
        Agent::new(&AgentConfig::cli("codex").expect("codex config")).expect("codex runtime");
    let invocation = agent.invoke(job_request()).expect("codex invocation");
    assert_eq!(invocation.program, "codex");
    assert_eq!(
        invocation.args,
        vec![
            "exec".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ]
    );
    assert_eq!(invocation.runtime_key, "codex");
    assert!(invocation.stdout_schema_json.is_none());
    let text = String::from_utf8(invocation.stdin).expect("utf8");
    assert!(text.contains("Execution envelope"));
}

#[test]
fn provider_mapper_supports_codex_approval_override() {
    let config = AgentConfig {
        command: "codex".to_string(),
        model: None,
        provider_options: ProviderOptions::Codex {
            sandbox: "workspace-write".to_string(),
            approval_policy: Some("on-request".to_string()),
        },
    };
    let agent = Agent::new(&config).expect("codex runtime");
    let invocation = agent.invoke(job_request()).expect("codex invocation");
    assert_eq!(invocation.program, "codex");
    assert_eq!(
        invocation.args,
        vec![
            "--ask-for-approval".to_string(),
            "on-request".to_string(),
            "exec".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ]
    );
}

#[test]
fn provider_mapper_supports_codex_model_override() {
    let agent = Agent::new(
        &AgentConfig::cli("codex")
            .expect("codex config")
            .with_model(Some("gpt-5.4")),
    )
    .expect("codex runtime");
    let invocation = agent.invoke(job_request()).expect("codex invocation");
    assert_eq!(
        invocation.args,
        vec![
            "exec".to_string(),
            "--model".to_string(),
            "gpt-5.4".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ]
    );
}

#[test]
fn provider_mapper_supports_mock_agent() {
    let agent = Agent::new(&AgentConfig::cli("mock-agent").expect("mock config"))
        .expect("mock-agent runtime");
    let invocation = agent.invoke(job_request()).expect("mock-agent invocation");
    assert_eq!(invocation.program, "mock-agent");
    assert_eq!(invocation.args, expected_job_args());
    assert_eq!(invocation.stdin, br#"{"schemaVersion":1}"#);
    assert!(invocation.stdout_schema_json.is_none());
}

#[test]
fn provider_mapper_supports_direct_activity_mode() {
    let agent = Agent::new(&AgentConfig::cli("mock-agent").expect("mock config"))
        .expect("mock-agent runtime");
    let invocation = agent
        .invoke(activity_request())
        .expect("mock-agent invocation");
    assert_eq!(invocation.program, "mock-agent");
    assert_eq!(invocation.args, expected_activity_args());
    assert_eq!(invocation.stdin, br#"{"schemaVersion":1}"#);
    assert!(invocation.stdout_schema_json.is_none());
}

#[test]
fn provider_mapper_uses_binary_basename_for_paths() {
    let agent = Agent::new(&AgentConfig::cli("/usr/local/bin/claude").expect("path config"))
        .expect("path-based runtime");
    let invocation = agent.invoke(job_request()).expect("path invocation");
    assert_eq!(invocation.program, "/usr/local/bin/claude");
    assert_eq!(
        invocation.args,
        vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
            "--no-session-persistence".to_string(),
        ]
    );
}

#[test]
fn provider_mapper_supports_claude_model_override() {
    let agent = Agent::new(
        &AgentConfig::cli("claude")
            .expect("claude config")
            .with_model(Some("sonnet-4.5")),
    )
    .expect("claude runtime");
    let invocation = agent.invoke(job_request()).expect("claude invocation");
    assert_eq!(
        invocation.args,
        vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
            "--no-session-persistence".to_string(),
            "--model".to_string(),
            "sonnet-4.5".to_string(),
        ]
    );
}

#[test]
fn provider_mapper_rejects_unsupported_provider() {
    let err = AgentConfig::cli("custom-agent")
        .err()
        .expect("unsupported provider must fail");
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
    assert_eq!(state, AgentResponseStatus::Success);
    assert_eq!(envelope.status, "success");
}

#[test]
fn stdin_payload_wraps_envelope_for_prompt_based_providers() {
    let agent =
        Agent::new(&AgentConfig::cli("codex").expect("codex config")).expect("codex runtime");
    let invocation = agent.invoke(job_request()).expect("codex invocation");
    let text = String::from_utf8(invocation.stdin).expect("utf8");
    assert!(text.contains("Execution envelope"));
    assert!(text.contains(r#"{"schemaVersion":1}"#));
}

#[test]
fn claude_runtime_declares_required_env_vars() {
    let agent =
        Agent::new(&AgentConfig::cli("claude").expect("claude config")).expect("claude runtime");
    let invocation = agent.invoke(job_request()).expect("claude invocation");
    assert_eq!(invocation.required_env_vars, &["HOME", "PATH"]);
    assert!(invocation.stdout_schema_json.is_none());
}

#[test]
fn claude_runtime_does_not_require_anthropic_api_key() {
    let agent =
        Agent::new(&AgentConfig::cli("claude").expect("claude config")).expect("claude runtime");
    let invocation = agent.invoke(job_request()).expect("claude invocation");
    assert!(
        !invocation.required_env_vars.contains(&"ANTHROPIC_API_KEY"),
        "ANTHROPIC_API_KEY must NOT be in Claude required_env_vars; HOME is sufficient"
    );
}

#[test]
fn protocol_parser_falls_back_to_failed_status_for_empty_stdout_with_stderr() {
    let exec = ExecutionResult {
        success: false,
        stdout: String::new(),
        stderr: "fatal: permission denied".to_string(),
        exit_code: Some(1),
        duration_ms: 1,
        output: None,
    };

    let (envelope, state) = parse_and_validate_response(&exec).expect("fallback response");
    assert_eq!(state, AgentResponseStatus::Failed);
    assert_eq!(envelope.status, "failed");
    assert!(
        envelope
            .error
            .as_ref()
            .expect("error")
            .message
            .contains("permission denied")
    );
}

#[test]
fn protocol_parser_falls_back_to_success_for_invalid_json_stdout_on_zero_exit() {
    let exec = ExecutionResult {
        success: true,
        stdout: "not-json".to_string(),
        stderr: "Reading prompt from stdin...".to_string(),
        exit_code: Some(0),
        duration_ms: 1,
        output: None,
    };

    let (envelope, state) = parse_and_validate_response(&exec).expect("fallback response");
    assert_eq!(state, AgentResponseStatus::Success);
    assert_eq!(envelope.status, "success");
    assert!(envelope.result.is_none());
}
