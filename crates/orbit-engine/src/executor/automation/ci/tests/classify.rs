use orbit_types::task::TaskStatus;
use serde_json::json;

use super::super::classify::classify;
use super::support::{TestHost, task};

fn host() -> TestHost {
    TestHost::new(vec![task("ORB-1", TaskStatus::InProgress)])
}

#[test]
fn capability_unavailable_blocks_with_the_preflight_evidence_and_stops_the_run() {
    let host = host();
    let error = classify(
        &host,
        &json!({
            "completed_task_ids": ["ORB-1"],
            "ci_evidence": {
                "collected": false,
                "capability": {
                    "available": true,
                    "authenticated": false,
                    "detail": "GitHub CLI is present but holds no usable credentials on this host",
                },
            },
        }),
    )
    .expect_err("capability_unavailable must stop the pipeline");

    let message = error.to_string();
    assert!(message.contains("capability_unavailable"), "{message}");
    assert!(message.contains("no usable credentials"), "{message}");
    assert!(message.contains("authenticated=false"), "{message}");
    // The one thing this must never be reported as.
    assert!(
        message.contains("must not be reported as one"),
        "the block reason has to say it is not a clean pass: {message}"
    );

    let updates = host.updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, "ORB-1");
    assert_eq!(updates[0].1.status, Some(TaskStatus::Blocked));
    assert_eq!(
        updates[0].1.status_event.as_deref(),
        Some("ci_capability_unavailable")
    );
    assert!(
        updates[0]
            .1
            .status_note
            .as_deref()
            .is_some_and(|note| note.contains("no usable credentials"))
    );
}

#[test]
fn no_current_failure_persists_an_evidenced_summary_and_never_claims_capability_loss() {
    let host = host();
    let output = classify(
        &host,
        &json!({
            "completed_task_ids": ["ORB-1"],
            "ci_evidence": {
                "collected": true,
                "capability": {"available": true, "authenticated": true, "detail": "ok"},
                "heads": [{"kind": "integration", "branch": "topic", "current_head_sha": "abc"}],
                "current_failures": [],
                "stale_or_superseded": [{
                    "url": "https://example.test/runs/1",
                    "reason": "superseded_by_success",
                }],
                "in_flight": [],
                "truncation": {"runs_listed": 3},
            },
        }),
    )
    .expect("classify");

    assert_eq!(output["outcome"], json!("no_current_failure"));
    assert_eq!(output["current_failure_count"], json!(0));

    let summary = output["detail"].as_str().expect("detail");
    assert!(
        summary.starts_with("Outcome: no_current_failure"),
        "{summary}"
    );
    assert!(summary.contains("integration `topic` at abc"), "{summary}");
    assert!(summary.contains("superseded_by_success"), "{summary}");
    assert!(
        summary.contains("not a capability_unavailable one"),
        "the two clean-looking endings must stay distinguishable: {summary}"
    );

    // Durable, because delivery reads the task record and not this output. It
    // is also what lets `git_commit` reach the shipped no-diff route.
    let updates = host.updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1.execution_summary.as_deref(), Some(summary));
    assert_eq!(updates[0].1.status, None, "a clean pass must not block");
    assert_eq!(host.get_task_status("ORB-1"), TaskStatus::InProgress);
}

#[test]
fn current_failures_carry_the_affected_workflows_forward_and_write_nothing() {
    let host = host();
    let output = classify(
        &host,
        &json!({
            "completed_task_ids": ["ORB-1"],
            "ci_evidence": {
                "collected": true,
                "capability": {"available": true, "authenticated": true, "detail": "ok"},
                "heads": [],
                "current_failures": [
                    {"workflow": "ci", "run_id": 1},
                    {"workflow": "lint", "run_id": 2},
                    {"workflow": "ci", "run_id": 3},
                ],
            },
        }),
    )
    .expect("classify");

    assert_eq!(output["outcome"], json!("current_failures"));
    assert_eq!(output["current_failure_count"], json!(3));
    assert_eq!(output["affected_workflows"], json!(["ci", "lint"]));
    assert!(host.updates().is_empty());
}

#[test]
fn a_snapshot_without_evidence_is_rejected_rather_than_read_as_clean() {
    let host = host();
    let error = classify(&host, &json!({"completed_task_ids": ["ORB-1"]}))
        .expect_err("missing evidence must not classify");
    assert!(error.to_string().contains("requires input.ci_evidence"));
    assert!(host.updates().is_empty());
}
