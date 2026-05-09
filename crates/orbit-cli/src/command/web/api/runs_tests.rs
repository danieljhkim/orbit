use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use orbit_common::utility::blob_store::BlobStore;
use orbit_core::{JobRunState, OrbitRuntime};
use serde_json::json;
use tower::ServiceExt;

use super::super::router;
use super::super::test_support::{body_json, seed_run, write_lines, write_replay_job};
use super::*;

async fn request_cancel(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/runs/{run_id}/cancel"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    router()
        .with_state(Arc::new(runtime))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_replay(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/runs/{run_id}/replay"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    router()
        .with_state(Arc::new(runtime))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_dashboard_run_events(runtime: OrbitRuntime, encoded_run_id: &str) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{encoded_run_id}/events"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn request_dashboard_run_logs(runtime: OrbitRuntime, encoded_run_id: &str) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{encoded_run_id}/logs"))
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
                "ts": "2026-05-08T04:12:20Z",
                "run_id": run_id,
                "body_kind": "run_started"
            })
            .to_string(),
            "malformed".to_string(),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step",
                "ts": "2026-05-08T04:12:21Z",
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
                "ts": "2026-05-08T04:12:22Z",
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

#[tokio::test]
async fn list_run_logs_returns_bounded_redacted_step_records() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-log-api";
    let mut stderr = String::from("first line\n");
    stderr.push_str("Authorization: Bearer sk-test-secret\n");
    for index in 0..200 {
        stderr.push_str(&format!("line {index}\n"));
    }
    let stderr_ref = seed_cli_invocation_audit(&runtime, run_id, stderr.as_bytes());

    let response = request_dashboard_run_logs(runtime, run_id).await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let rows = payload.as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["run_id"], run_id);
    assert_eq!(rows[0]["event_id"], "evt-cli");
    assert_eq!(rows[0]["step_id"], "implement");
    assert_eq!(rows[0]["step_index"], 0);
    assert_eq!(rows[0]["provider"], "codex");
    assert_eq!(rows[0]["stderr_blob_ref"], stderr_ref);
    assert_eq!(rows[0]["exit_code"], 0);
    assert_eq!(rows[0]["timed_out"], false);
    assert_eq!(rows[0]["duration_ms"], 123);
    let preview = rows[0]["stderr_preview"].as_str().expect("stderr preview");
    assert!(preview.contains("[REDACTED_AUTH]"));
    assert!(!preview.contains("sk-test-secret"));
    assert_eq!(rows[0]["stderr_truncated"], true);
}

#[tokio::test]
async fn list_run_events_rejects_path_traversal_id() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_dashboard_run_events(runtime, "..%2F..%2Fetc%2Fpasswd").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_run_events_rejects_id_with_slashes() {
    let cases = [
        ("jrun%2F1", "literal slash"),
        ("jrun%5C1", "backslash"),
        (".jrun-1", "leading dot"),
        ("jrun%00nul", "nul byte"),
    ];

    for (encoded_run_id, label) in cases {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");

        let response = request_dashboard_run_events(runtime, encoded_run_id).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
    }
}

#[tokio::test]
async fn list_run_events_accepts_valid_run_id() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-1";
    let audit_dir = runtime.data_root().join("state/audit/v2_loop");
    std::fs::create_dir_all(&audit_dir).expect("create audit dir");
    write_lines(
        &audit_dir.join(format!("{run_id}.jsonl")),
        &[json!({
            "schemaVersion": 1,
            "event_type": "step.started",
            "event_id": "evt-step-started",
            "run_id": run_id,
            "body_kind": "step_started"
        })
        .to_string()],
    );

    let response = request_dashboard_run_events(runtime, run_id).await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let events = payload.as_array().expect("events array");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["run_id"], run_id);
    assert_eq!(events[0]["body_kind"], "step_started");
}

#[tokio::test]
async fn cancel_run_endpoint_cancels_pending_run() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run = seed_run(
        &runtime,
        "jrun-web-cancel-pending",
        "web_cancel_pending",
        JobRunState::Pending,
    );

    let response =
        request_cancel(runtime.clone(), &run.run_id, Some("http://localhost:3000")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["run_id"], run.run_id);
    assert_eq!(payload["previous_state"], "pending");
    assert_eq!(payload["final_state"], "cancelled");
    assert_eq!(payload["signal_attempted"], false);
    assert_eq!(payload["signal_outcome"], Value::Null);
    let stored = runtime.show_job_run(&run.run_id).expect("show cancelled");
    assert_eq!(stored.state, JobRunState::Cancelled);
}

#[tokio::test]
async fn cancel_run_endpoint_rejects_terminal_run_without_mutating_bundle() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run = seed_run(
        &runtime,
        "jrun-web-cancel-terminal",
        "web_cancel_terminal",
        JobRunState::Success,
    );
    let before = runtime.show_job_run(&run.run_id).expect("show before");

    let response =
        request_cancel(runtime.clone(), &run.run_id, Some("http://localhost:3000")).await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("cannot cancel job run"))
    );
    let after = runtime.show_job_run(&run.run_id).expect("show after");
    assert_eq!(after, before);
}

#[tokio::test]
async fn cancel_run_endpoint_applies_localhost_origin_guard() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run = seed_run(
        &runtime,
        "jrun-web-cancel-origin",
        "web_cancel_origin",
        JobRunState::Pending,
    );

    let response = request_cancel(runtime.clone(), &run.run_id, Some("https://example.test")).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(stored.state, JobRunState::Pending);
}

#[tokio::test]
async fn replay_run_endpoint_returns_new_run_id_and_lineage() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let job_path = write_replay_job(&runtime, "web_replay_success");
    let source = runtime
        .run_job_v2_from_yaml(&job_path, json!({ "seconds": 0 }), None)
        .expect("source run succeeds");

    let response = request_replay(
        runtime.clone(),
        &source.run_id,
        Some("http://localhost:3000"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let new_run_id = payload["run_id"].as_str().expect("new run id");
    assert_ne!(new_run_id, source.run_id);
    let stored = runtime.show_job_run(new_run_id).expect("show replay");
    assert_eq!(stored.state, JobRunState::Success);
    assert_eq!(
        stored.retry_source_run_id.as_deref(),
        Some(source.run_id.as_str())
    );
    let list_response = router()
        .with_state(Arc::new(runtime.clone()))
        .oneshot(
            Request::builder()
                .uri("/job-runs?limit=10")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_payload = body_json(list_response).await;
    assert!(
        list_payload
            .as_array()
            .expect("runs array")
            .iter()
            .any(|run| run["run_id"].as_str() == Some(new_run_id))
    );

    let detail = job_run_detail_to_json(&runtime, &stored);
    assert_eq!(
        detail["run"]["retry_source_run_id"].as_str(),
        Some(source.run_id.as_str())
    );
}

#[tokio::test]
async fn replay_run_endpoint_returns_4xx_when_current_job_is_deleted() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let job_path = write_replay_job(&runtime, "web_replay_deleted");
    let source = runtime
        .run_job_v2_from_yaml(&job_path, json!({ "seconds": 0 }), None)
        .expect("source run succeeds");
    std::fs::remove_file(&job_path).expect("delete job yaml");

    let response = request_replay(
        runtime.clone(),
        &source.run_id,
        Some("http://localhost:3000"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("job not found"))
    );
}

#[test]
fn run_detail_uses_v2_audit_steps_when_step_bundle_is_empty() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-web-audit-step";
    let audit_dir = runtime.data_root().join("state/audit/v2_loop");
    std::fs::create_dir_all(&audit_dir).expect("create audit dir");
    write_lines(
        &audit_dir.join(format!("{run_id}.jsonl")),
        &[
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-started",
                "ts": "2026-04-28T00:00:01Z",
                "run_id": run_id,
                "agent_identity": "system",
                "body_kind": "step_started",
                "step_id": "build"
            })
            .to_string(),
            json!({
                "schemaVersion": 1,
                "event_type": "step.finished",
                "event_id": "evt-step-finished",
                "ts": "2026-04-28T00:00:03Z",
                "run_id": run_id,
                "agent_identity": "system",
                "body_kind": "step_finished",
                "step_id": "build",
                "outcome": "success"
            })
            .to_string(),
        ],
    );
    let scheduled_at = chrono::DateTime::parse_from_rfc3339("2026-04-28T00:00:00Z")
        .expect("parse scheduled")
        .with_timezone(&Utc);
    let run = orbit_core::JobRun {
        run_id: run_id.to_string(),
        job_id: "job-web".to_string(),
        attempt: 1,
        state: JobRunState::Success,
        scheduled_at,
        started_at: Some(scheduled_at),
        finished_at: Some(scheduled_at),
        duration_ms: Some(2_000),
        created_at: scheduled_at,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        steps: Vec::new(),
    };

    let detail = job_run_detail_to_json(&runtime, &run);
    let steps = detail["steps"].as_array().expect("steps array");

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["step_index"], 0);
    assert_eq!(steps[0]["target_type"], "activity");
    assert_eq!(steps[0]["target_id"], "build");
    assert_eq!(steps[0]["state"], "success");
    assert_eq!(steps[0]["duration_ms"], 2_000);
}
