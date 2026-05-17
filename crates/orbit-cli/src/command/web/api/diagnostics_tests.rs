use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use orbit_core::OrbitRuntime;
use serde_json::json;
use tempfile::tempdir;
use tower::ServiceExt;

use super::super::router;
use super::super::test_support::{body_json, write_lines};
use super::*;

async fn request_dashboard_errors(runtime: OrbitRuntime) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .uri("/api/diagnostics/errors?limit=10")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

fn seed_cli_invocation_audit(runtime: &OrbitRuntime, run_id: &str, stderr: &[u8]) -> String {
    let audit_root = runtime.data_root().join("state").join("audit");
    let blob_store = BlobStore::new(audit_root.join("blobs"));
    let stdout_ref = blob_store
        .write(b"normal output\n")
        .expect("write stdout blob");
    let stderr_ref = blob_store.write(stderr).expect("write stderr blob");
    let audit_dir = audit_root.join("v2_loop");
    std::fs::create_dir_all(&audit_dir).expect("create audit dir");
    write_lines(
        &audit_dir.join(format!("{run_id}.jsonl")),
        &[
            json!({
                "schemaVersion": 1,
                "event_type": "run.started",
                "event_id": "evt-run",
                "ts": "2099-05-08T04:12:20Z",
                "run_id": run_id,
                "body_kind": "run_started"
            })
            .to_string(),
            "malformed".to_string(),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step",
                "ts": "2099-05-08T04:12:21Z",
                "run_id": run_id,
                "parent_event_id": "evt-run",
                "body_kind": "step_started",
                "step_id": "implement"
            })
            .to_string(),
            json!({
                "schemaVersion": 1,
                "event_type": "cli.invocation.finished",
                "event_id": "evt-cli",
                "ts": "2099-05-08T04:12:22Z",
                "run_id": run_id,
                "parent_event_id": "evt-step",
                "body_kind": "cli_invocation_finished",
                "provider": "codex",
                "stdout_blob_ref": stdout_ref,
                "stderr_blob_ref": stderr_ref,
                "exit_code": 0,
                "timed_out": false,
                "duration_ms": 123
            })
            .to_string(),
        ],
    );
    stderr_ref
}

#[test]
fn diagnostics_metrics_values_adapt_invocation_records() {
    let ts = chrono::DateTime::parse_from_rfc3339("2026-05-05T03:29:45Z")
        .expect("parse timestamp")
        .with_timezone(&Utc);
    let rows = diagnostics_metrics_values(vec![InvocationRecord {
        id: 7,
        ts,
        job_run_id: "jrun-1".to_string(),
        activity_id: "implement_one".to_string(),
        agent: "codex".to_string(),
        model: Some("gpt-5.5".to_string()),
        slot: None,
        duration_ms: 1234,
        input_tokens: 100,
        cache_read_tokens: 0,
        cache_create_tokens: 0,
        output_tokens: 23,
        total_tokens: 123,
        tool_call_count: 4,
        task_ids: vec!["T20260505-1".to_string()],
        tool_calls: Vec::new(),
    }]);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["step"], "implement_one");
    assert_eq!(rows[0]["actor_identity"], "codex / gpt-5.5");
    assert_eq!(rows[0]["token_usage"], 123);
    assert_eq!(rows[0]["tool_invocations"], 4);
    assert_eq!(rows[0]["step_duration_ms"], 1234);
    assert_eq!(rows[0]["task_id"], "T20260505-1");
}

#[test]
fn diagnostics_friction_row_extracts_failed_cli_stderr_and_step() {
    let dir = tempdir().expect("tempdir");
    let blob_store = BlobStore::new(dir.path());
    let stderr_ref = blob_store.write(b"command failed\n").expect("write blob");

    let step = json!({
        "event_id": "evt-step",
        "body_kind": "step_started",
        "step_id": "implement_one"
    });
    let activity = json!({
        "event_id": "evt-activity",
        "body_kind": "activity_started",
        "parent_event_id": "evt-step"
    });
    let event = json!({
        "event_id": "evt-cli",
        "ts": "2026-05-05T03:29:45Z",
        "run_id": "jrun-1",
        "agent_identity": "system",
        "body_kind": "cli_invocation_finished",
        "parent_event_id": "evt-activity",
        "provider": "codex",
        "exit_code": 1,
        "stderr_blob_ref": stderr_ref,
        "timed_out": false
    });
    let events_by_id = HashMap::from([
        ("evt-step".to_string(), step),
        ("evt-activity".to_string(), activity),
        ("evt-cli".to_string(), event.clone()),
    ]);

    let row = diagnostics_friction_row(&blob_store, &events_by_id, &event, "2026-05").expect("row");

    assert_eq!(row["step"], "implement_one");
    assert_eq!(row["command"], "codex");
    assert_eq!(row["exit_code"], 1);
    assert_eq!(row["stderr"], "command failed\n");
}

#[tokio::test]
async fn diagnostics_errors_include_codex_style_stderr_rows() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-error-api";
    // The endpoint merges global process-log errors with fixture audit errors before truncating.
    let stderr = b"2099-05-08T04:12:22.346005Z ERROR codex_core::session: failed to record rollout items\nordinary stderr\nERROR codex_core::tools::router: apply_patch verification failed\n";
    let stderr_ref = seed_cli_invocation_audit(&runtime, run_id, stderr);

    let response = request_dashboard_errors(runtime).await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let rows = payload.as_array().expect("rows");
    let agent_rows = rows
        .iter()
        .filter(|row| row["source"] == "agent-stderr" && row["job_run"] == run_id)
        .collect::<Vec<_>>();
    assert_eq!(agent_rows.len(), 2);
    assert_eq!(agent_rows[0]["step"], "implement");
    assert_eq!(agent_rows[0]["step_index"], 0);
    assert_eq!(agent_rows[0]["provider"], "codex");
    assert_eq!(agent_rows[0]["blob_ref"], stderr_ref);
    assert!(rows.iter().any(|row| {
        row["message"]
            .as_str()
            .is_some_and(|message| message.contains("apply_patch verification failed"))
    }));
}

#[test]
fn global_error_rows_include_process_log_errors() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("orbit.log.jsonl");
    write_lines(
        &path,
        &[
            json!({
                "timestamp": "2026-05-08T04:00:00Z",
                "level": "INFO",
                "target": "orbit.test",
                "fields": { "message": "ignored" }
            })
            .to_string(),
            json!({
                "timestamp": "2026-05-08T04:01:00Z",
                "level": "ERROR",
                "target": "orbit.test",
                "fields": { "message": "process failed" }
            })
            .to_string(),
        ],
    );

    let rows = global_error_rows_from_path(&path, 10).expect("rows");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["ts"], "2026-05-08T04:01:00Z");
    assert_eq!(rows[0]["source"], "process");
    assert!(
        rows[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("process failed"))
    );
}

#[test]
fn parse_structured_error_line_ignores_unstructured_error_words() {
    assert!(parse_structured_error_line("this has ERROR but no shape", "").is_none());
    let parsed = parse_structured_error_line(
        "2026-05-08T04:12:22.346005Z ERROR codex_core::session: failed",
        "",
    )
    .expect("parsed");
    assert_eq!(parsed.ts, "2026-05-08T04:12:22.346005Z");
    assert_eq!(parsed.target, "codex_core::session");
    assert_eq!(parsed.message, "failed");
}
