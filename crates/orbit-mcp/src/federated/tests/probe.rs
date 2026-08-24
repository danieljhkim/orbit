//! Delivery budget and post-dispatch classification.
//!
//! The fake destination here is a child process that holds the pipe open and
//! never writes a line, so a real [`DestinationSession`] runs its own timeout
//! path without an SSH host: everything the mux sends is accepted and nothing
//! is ever answered.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use orbit_common::OrbitError;
use serde_json::json;

use super::super::probe::{DestinationSession, RoutedSession, SshRoutedSession};
use super::fixtures::{OWNER_MACHINE, destination};

/// A destination that takes the request and stops there.
fn stalled_session(budget: Duration) -> DestinationSession {
    let child = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a stalled destination");
    DestinationSession::start(destination("orbit-owner", OWNER_MACHINE), child, budget)
        .expect("start a session against the stalled destination")
}

#[test]
fn a_stall_before_the_tool_call_is_an_unreachable_destination() {
    let mut session = stalled_session(Duration::from_millis(50));

    for (phase, error) in [
        ("initialize", session.handshake().err()),
        ("discovery", session.discover_workspaces().err()),
        ("tools/list", session.list_tools().err()),
    ] {
        let error = error.unwrap_or_else(|| panic!("{phase} cannot complete against a stall"));
        assert!(
            matches!(error, OrbitError::UnreachableDestination(_)),
            "nothing the caller asked for has run yet at {phase}: {error}"
        );
    }
}

#[test]
fn a_stall_after_the_tool_call_is_dispatched_is_outcome_unknown() {
    let mut session = stalled_session(Duration::from_millis(50));

    let error = session
        .call_tool("orbit.task.add", json!({ "title": "remote write" }))
        .expect_err("the destination never answers the dispatched call");

    match error {
        OrbitError::OutcomeUnknown {
            mcp_call_id,
            message,
        } => {
            assert!(
                mcp_call_id.starts_with(&format!("{OWNER_MACHINE}/orbit.task.add#")),
                "the ambiguous call names the destination-facing request: {mcp_call_id}"
            );
            assert!(
                message.contains("may have completed"),
                "the message must not read as a delivery miss: {message}"
            );
        }
        other => panic!(
            "a written tools/call that timed out may already have committed remotely, so it \
             cannot be reported as an unreachable destination: {other}"
        ),
    }
}

#[test]
fn routed_delivery_does_not_inherit_an_exhausted_probe_budget() {
    let delivery = Duration::from_millis(300);

    // A zero probe budget is the limit case of what SSH setup, the handshake,
    // discovery, and tools/list leave behind on a slow destination.
    let mut exhausted = stalled_session(Duration::ZERO);
    let started = Instant::now();
    exhausted
        .list_tools()
        .expect_err("an exhausted probe budget gives up at once");
    assert!(
        started.elapsed() < delivery,
        "the probe budget really is spent, so the contrast below is meaningful"
    );

    let mut routed = SshRoutedSession::new(stalled_session(Duration::ZERO), delivery);
    let started = Instant::now();
    let error = routed
        .call_tool("orbit.command.exec", json!({ "command": "make ci-lint" }))
        .expect_err("the destination never answers the dispatched call");
    let waited = started.elapsed();

    assert!(
        waited >= delivery,
        "the tool call gets its own budget rather than what classification left over: \
         gave up after {waited:?}"
    );
    assert!(
        matches!(error, OrbitError::OutcomeUnknown { .. }),
        "{error}"
    );
}
