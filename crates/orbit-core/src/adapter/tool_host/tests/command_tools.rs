//! [ORB-10711, ADR-0351] Claim-gated remote command execution.

use orbit_common::OrbitError;
use serde_json::{Value, json};

use super::super::test_support::{
    managed_tool_env_guard, run_tool_as_operator, test_runtime, unmanaged_tool_env_guard,
};
use crate::OrbitRuntime;

/// Acquire the workspace claim as `actor` and return its token.
fn acquire_claim(runtime: &OrbitRuntime, actor: &str) -> String {
    let result = run_tool_as_operator(
        runtime,
        "orbit.workspace.claim.acquire",
        json!({ "model": actor }),
    )
    .expect("acquire workspace claim");
    assert_eq!(result["acquired"], json!(true));
    result["claim_token"]
        .as_str()
        .expect("claim grant carries a token")
        .to_string()
}

#[test]
fn claim_holder_executes_and_receives_stdout_stderr_and_exit_status() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let token = acquire_claim(&runtime, "claude");

    let result = run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": ["echo", "hello-from-command-exec"],
            "working_directory": repo_root.display().to_string(),
            "claim_token": token,
            "model": "claude",
        }),
    )
    .expect("the claim holder must be able to execute a command");

    assert_eq!(result["success"], json!(true));
    assert_eq!(result["exit_code"], json!(0));
    assert!(
        result["stdout"]
            .as_str()
            .expect("stdout is a string")
            .contains("hello-from-command-exec"),
        "unexpected stdout: {result}"
    );
}

#[test]
fn operator_without_the_claim_is_refused() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    acquire_claim(&runtime, "claude");

    let error = run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": ["echo", "hello"],
            "working_directory": repo_root.display().to_string(),
            "model": "codex",
        }),
    )
    .expect_err("a caller without the holder's token must be refused");

    let OrbitError::WorkspaceClaimHeld(claim) = &error else {
        panic!("expected WorkspaceClaimHeld, got {error:?}");
    };
    assert_eq!(claim.operation, "orbit.command.exec");
    assert_eq!(claim.holder, "claude");
}

#[test]
fn shell_string_argv_is_rejected() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();

    let error = run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": "echo hello",
            "working_directory": repo_root.display().to_string(),
            "model": "codex",
        }),
    )
    .expect_err("a shell string must be rejected, not spawned or interpreted");

    let OrbitError::InvalidInput(message) = &error else {
        panic!("expected InvalidInput, got {error:?}");
    };
    assert!(
        message.contains("shell string"),
        "unexpected message: {message}"
    );
}

#[test]
fn empty_argv_is_rejected() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();

    let error = run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": [],
            "working_directory": repo_root.display().to_string(),
            "model": "codex",
        }),
    )
    .expect_err("an empty argv names no program to run");

    assert!(matches!(error, OrbitError::InvalidInput(_)), "{error:?}");
}

#[test]
fn managed_run_environment_denies_command_exec() {
    let _env = managed_tool_env_guard("jrun-test-managed-command-exec");
    let (_root, runtime, repo_root) = test_runtime();

    let error = run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": ["echo", "hello"],
            "working_directory": repo_root.display().to_string(),
            "model": "codex",
        }),
    )
    .expect_err("a managed run must not execute remote commands");

    assert!(
        matches!(error, OrbitError::CapabilityDenied(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("managed runs cannot execute"));
}

#[test]
fn audit_record_carries_argv_working_directory_caller_and_workspace() {
    let _env = unmanaged_tool_env_guard();
    let (_root, runtime, repo_root) = test_runtime();
    let token = acquire_claim(&runtime, "claude");

    run_tool_as_operator(
        &runtime,
        "orbit.command.exec",
        json!({
            "argv": ["echo", "audited-command"],
            "working_directory": repo_root.display().to_string(),
            "claim_token": token,
            "model": "claude",
        }),
    )
    .expect("claim holder executes");

    let events = runtime
        .list_audit_events(None, None, None, None, 200)
        .expect("read audit events");
    let event = events
        .iter()
        .find(|event| event.command == "command.exec")
        .expect("command execution is audited");
    let payload: Value = event
        .arguments_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .expect("audit payload is recorded JSON");

    assert_eq!(payload["argv"], json!(["echo", "audited-command"]));
    assert_eq!(
        payload["working_directory"],
        json!(repo_root.display().to_string())
    );
    assert_eq!(payload["caller"], json!("claude"));
    assert_eq!(payload["workspace"], json!(repo_root.display().to_string()));
}
