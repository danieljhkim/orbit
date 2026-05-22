//! Sibling tests for `run_audit.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use crate::OrbitRuntime;
use orbit_common::utility::blob_store::BlobStore;

use serde_json::json;

#[test]
fn collect_run_cli_invocations_derives_step_ids_from_parent_chain() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let audit_root = runtime.data_root().join("state").join("audit");
    let blob_store = BlobStore::new(audit_root.join("blobs"));

    let stdout_one = blob_store.write(b"one stdout\n").expect("write stdout one");
    let stderr_one = blob_store.write(b"one stderr\n").expect("write stderr one");
    let stdout_two = blob_store.write(b"two stdout\n").expect("write stdout two");

    let jsonl_dir = audit_root.join("v2_loop");
    std::fs::create_dir_all(&jsonl_dir).expect("create jsonl dir");
    let run_id = "jrun-test";
    let events = [
        json!({
            "schemaVersion": 1,
            "event_type": "run.started",
            "event_id": "evt-run-started",
            "ts": "2026-04-26T07:00:00Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "body_kind": "run_started",
            "job_name": "test-job"
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "step.started",
            "event_id": "evt-step-one",
            "ts": "2026-04-26T07:00:01Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-run-started",
            "body_kind": "step_started",
            "step_id": "implement_one"
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "activity.started",
            "event_id": "evt-activity-one",
            "ts": "2026-04-26T07:00:02Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-step-one",
            "body_kind": "activity_started",
            "activity_name": "worker",
            "activity_type": "agent_loop"
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "cli.invocation.finished",
            "event_id": "evt-cli-one",
            "ts": "2026-04-26T07:00:03Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-activity-one",
            "body_kind": "cli_invocation_finished",
            "provider": "codex",
            "exit_code": 0,
            "duration_ms": 10,
            "stdout_blob_ref": stdout_one,
            "stderr_blob_ref": stderr_one,
            "harness_version": null,
            "timed_out": false
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "step.started",
            "event_id": "evt-step-two",
            "ts": "2026-04-26T07:00:04Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-run-started",
            "body_kind": "step_started",
            "step_id": "review"
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "activity.started",
            "event_id": "evt-activity-two",
            "ts": "2026-04-26T07:00:05Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-step-two",
            "body_kind": "activity_started",
            "activity_name": "reviewer",
            "activity_type": "agent_loop"
        }),
        json!({
            "schemaVersion": 1,
            "event_type": "cli.invocation.finished",
            "event_id": "evt-cli-two",
            "ts": "2026-04-26T07:00:06Z",
            "run_id": run_id,
            "agent_identity": "codex",
            "parent_event_id": "evt-activity-two",
            "body_kind": "cli_invocation_finished",
            "provider": "claude",
            "exit_code": 0,
            "duration_ms": 20,
            "stdout_blob_ref": stdout_two,
            "stderr_blob_ref": null,
            "harness_version": null,
            "timed_out": false
        }),
    ];
    let jsonl = events
        .iter()
        .map(|event| serde_json::to_string(event).expect("serialize event"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        jsonl_dir.join(format!("{run_id}.jsonl")),
        format!("{jsonl}\n"),
    )
    .expect("write jsonl");

    let records = runtime
        .collect_run_cli_invocations(run_id)
        .expect("collect records");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].run_id, run_id);
    assert_eq!(records[0].event_id, "evt-cli-one");
    assert_eq!(records[0].step_id.as_deref(), Some("implement_one"));
    assert_eq!(records[0].step_index, Some(0));
    assert_eq!(records[0].provider.as_deref(), Some("codex"));
    assert_eq!(records[0].stdout, "one stdout\n");
    assert_eq!(records[0].stderr, "one stderr\n");
    assert_eq!(records[0].exit_code, Some(0));
    assert!(!records[0].timed_out);
    assert_eq!(records[0].duration_ms, Some(10));
    assert_eq!(records[1].step_id.as_deref(), Some("review"));
    assert_eq!(records[1].step_index, Some(1));
    assert_eq!(records[1].provider.as_deref(), Some("claude"));
    assert_eq!(records[1].stdout, "two stdout\n");
    assert_eq!(records[1].stderr, "");
}

#[test]
fn missing_run_audit_file_returns_no_cli_invocations() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let records = runtime
        .collect_run_cli_invocations("jrun-missing")
        .expect("collect records");
    assert!(records.is_empty());
}

#[test]
fn collect_run_audit_steps_reads_step_finished_error_message_and_tolerates_absence() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let audit_root = runtime.data_root().join("state").join("audit");
    let jsonl_dir = audit_root.join("v2_loop");
    std::fs::create_dir_all(&jsonl_dir).expect("create jsonl dir");
    let run_id = "jrun-step-errors";
    let events = [
        json!({
            "event_id": "evt-step-one",
            "ts": "2026-04-26T07:00:01Z",
            "run_id": run_id,
            "body_kind": "step_started",
            "step_id": "plan"
        }),
        json!({
            "event_id": "evt-step-one-finished",
            "ts": "2026-04-26T07:00:02Z",
            "run_id": run_id,
            "body_kind": "step_finished",
            "step_id": "plan",
            "outcome": "error",
            "error_message": "planning duel failed"
        }),
        json!({
            "event_id": "evt-step-two",
            "ts": "2026-04-26T07:00:03Z",
            "run_id": run_id,
            "body_kind": "step_started",
            "step_id": "review"
        }),
        json!({
            "event_id": "evt-step-two-finished",
            "ts": "2026-04-26T07:00:04Z",
            "run_id": run_id,
            "body_kind": "step_finished",
            "step_id": "review",
            "outcome": "success"
        }),
    ];
    let jsonl = events
        .iter()
        .map(|event| serde_json::to_string(event).expect("serialize event"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        jsonl_dir.join(format!("{run_id}.jsonl")),
        format!("{jsonl}\n"),
    )
    .expect("write jsonl");

    let steps = runtime
        .collect_run_audit_steps(run_id)
        .expect("collect steps");

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].step_id, "plan");
    assert_eq!(steps[0].outcome.as_deref(), Some("error"));
    assert_eq!(
        steps[0].error_message.as_deref(),
        Some("planning duel failed")
    );
    assert_eq!(steps[1].step_id, "review");
    assert_eq!(steps[1].outcome.as_deref(), Some("success"));
    assert_eq!(steps[1].error_message, None);
}

#[test]
fn malformed_jsonl_and_missing_blobs_are_tolerated() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let audit_root = runtime.data_root().join("state").join("audit");
    let jsonl_dir = audit_root.join("v2_loop");
    std::fs::create_dir_all(&jsonl_dir).expect("create jsonl dir");
    let run_id = "jrun-tolerant";
    std::fs::write(
        jsonl_dir.join(format!("{run_id}.jsonl")),
        format!(
            "{}\nnot-json\n{}\n",
            json!({
                "event_id": "evt-step",
                "ts": "2026-04-26T07:00:01Z",
                "run_id": run_id,
                "body_kind": "step_started",
                "step_id": "implement"
            }),
            json!({
                "event_id": "evt-cli",
                "ts": "2026-04-26T07:00:02Z",
                "run_id": run_id,
                "parent_event_id": "evt-step",
                "body_kind": "cli_invocation_finished",
                "provider": "codex",
                "exit_code": 1,
                "duration_ms": 42,
                "stdout_blob_ref": "aa/missing",
                "stderr_blob_ref": "error:writer-failed",
                "timed_out": true
            })
        ),
    )
    .expect("write jsonl");

    let events = runtime
        .collect_run_audit_events(run_id)
        .expect("collect events");
    assert_eq!(events.len(), 2);

    let records = runtime
        .collect_run_cli_invocations(run_id)
        .expect("collect records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].step_index, Some(0));
    assert_eq!(records[0].stdout, "");
    assert_eq!(records[0].stderr, "");
    assert_eq!(records[0].exit_code, Some(1));
    assert!(records[0].timed_out);
}
