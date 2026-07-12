//! Loopback worker-client tests [ORB-10146]: request-body shaping, response
//! parsing, terminal classification, the poll-until-terminal timeout, and the
//! daemon-down failure path — all without a live daemon.

use std::cell::Cell;
use std::time::Duration;

use crate::qa::worker::{
    WorkerClient, WorkerError, WorkerRunRequest, WorkerRunStatus, await_terminal,
    build_invoke_body, is_terminal, parse_run_status, parse_submit_response,
};

fn request(provider: &str, max_turns: Option<u32>) -> WorkerRunRequest {
    WorkerRunRequest {
        prompt: "validate the new feature".to_string(),
        provider: provider.to_string(),
        model: "opus".to_string(),
        cwd: "/repo".to_string(),
        wall_clock_secs: 7200,
        max_turns,
        serialization_key: Some("qa-sweep:polaris".to_string()),
    }
}

#[test]
fn invoke_body_carries_prompt_provider_cwd_and_limits() {
    let body = build_invoke_body(&request("claude", Some(150)));
    assert_eq!(body["prompt"], "validate the new feature");
    assert_eq!(body["provider"], "claude");
    assert_eq!(body["model"], "opus");
    assert_eq!(body["cwd"], "/repo");
    assert_eq!(body["limits"]["wall_clock_secs"], 7200);
    assert_eq!(body["limits"]["max_turns"], 150);
    assert_eq!(body["serialization_key"], "qa-sweep:polaris");
}

#[test]
fn invoke_body_omits_max_turns_when_unset() {
    // Codex rejects `max_turns`; the caller passes None there.
    let body = build_invoke_body(&request("codex", None));
    assert!(body["limits"].get("max_turns").is_none());
    assert_eq!(body["limits"]["wall_clock_secs"], 7200);
}

#[test]
fn parses_run_id_from_submit_response() {
    let run_id =
        parse_submit_response(r#"{"run_id":"abc-123","status":"queued"}"#).expect("run id");
    assert_eq!(run_id, "abc-123");
}

#[test]
fn submit_response_without_run_id_is_a_bad_response() {
    match parse_submit_response(r#"{"status":"queued"}"#) {
        Err(WorkerError::BadResponse(_)) => {}
        other => panic!("expected BadResponse, got {other:?}"),
    }
}

#[test]
fn submit_response_that_is_not_json_is_a_bad_response() {
    assert!(matches!(
        parse_submit_response("<html>502</html>"),
        Err(WorkerError::BadResponse(_))
    ));
}

#[test]
fn parses_status_and_result_text_from_run_record() {
    let status = parse_run_status(r#"{"status":"ok","result":{"result":"done","completed":true}}"#)
        .expect("status");
    assert_eq!(status.status, "ok");
    assert_eq!(status.report_text.as_deref(), Some("done"));
}

#[test]
fn parses_bare_string_result_on_dispatch_failure() {
    let status =
        parse_run_status(r#"{"status":"error","result":"claude not found"}"#).expect("status");
    assert_eq!(status.status, "error");
    assert_eq!(status.report_text.as_deref(), Some("claude not found"));
}

#[test]
fn run_status_missing_status_is_a_bad_response() {
    assert!(matches!(
        parse_run_status(r#"{"result":{"result":"x"}}"#),
        Err(WorkerError::BadResponse(_))
    ));
}

#[test]
fn terminal_statuses_are_recognized() {
    for status in [
        "ok",
        "error",
        "timeout",
        "max_turns",
        "cost_exceeded",
        "cancelled",
        "interrupted",
    ] {
        assert!(is_terminal(status), "{status} should be terminal");
    }
    for status in ["queued", "running", "unknown"] {
        assert!(!is_terminal(status), "{status} should not be terminal");
    }
}

#[test]
fn await_terminal_returns_immediately_on_a_terminal_status() {
    let terminal = await_terminal(
        Duration::from_secs(5),
        Duration::from_millis(1),
        "r",
        || {
            Ok(WorkerRunStatus {
                status: "ok".to_string(),
                report_text: Some("report".to_string()),
            })
        },
    )
    .expect("terminal");
    assert_eq!(terminal.status, "ok");
    assert_eq!(terminal.report_text.as_deref(), Some("report"));
}

#[test]
fn await_terminal_times_out_on_a_run_that_never_finishes() {
    let polls = Cell::new(0u32);
    let result = await_terminal(
        Duration::from_millis(40),
        Duration::from_millis(2),
        "run-x",
        || {
            polls.set(polls.get() + 1);
            Ok(WorkerRunStatus {
                status: "running".to_string(),
                report_text: None,
            })
        },
    );
    match result {
        Err(WorkerError::Timeout { run_id, .. }) => assert_eq!(run_id, "run-x"),
        other => panic!("expected Timeout, got {other:?}"),
    }
    assert!(polls.get() >= 1, "polled at least once");
}

#[test]
fn await_terminal_propagates_a_poll_error() {
    let result = await_terminal(
        Duration::from_secs(5),
        Duration::from_millis(1),
        "r",
        || Err(WorkerError::Poll("boom".to_string())),
    );
    assert!(matches!(result, Err(WorkerError::Poll(_))));
}

#[test]
fn submit_to_a_dead_daemon_reports_unreachable() {
    // Port 9 (discard) is not served on loopback; the connection is refused.
    let client = WorkerClient::new("http://127.0.0.1:9").expect("client");
    let result = client.submit(&request("claude", Some(150)));
    // A connection refusal classifies as an unreachable daemon; any sandbox
    // that blocks the connect still surfaces a typed error, never a hang.
    assert!(result.is_err(), "dead daemon must be an error");
    if let Err(error) = &result {
        assert!(
            matches!(error, WorkerError::Unreachable(_)),
            "expected Unreachable, got {error:?}"
        );
    }
}
