//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use orbit_common::utility::blob_store::BlobStore;
use orbit_core::command::job::JobRunListParams;
use orbit_core::command::task::TaskAddParams;
use orbit_core::{JobRunState, OrbitRuntime, TaskStatus, V2AuditEventInsertParams};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::super::runs::*;
use super::test_support::{body_json, seed_run, write_replay_job, write_seeded_run};

async fn request_cancel(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/runs/{run_id}/cancel"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_resume(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/job-runs/{run_id}/resume"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_dashboard_run_events(runtime: OrbitRuntime, encoded_run_id: &str) -> Response {
    request_dashboard_run_events_query(runtime, encoded_run_id, "").await
}

async fn request_dashboard_run_events_query(
    runtime: OrbitRuntime,
    encoded_run_id: &str,
    query: &str,
) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .uri(format!("/api/runs/{encoded_run_id}/events{query}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn request_dashboard_run_logs(runtime: OrbitRuntime, encoded_run_id: &str) -> Response {
    Router::new()
        .nest("/api", router())
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
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
    seed_v2_audit_events(
        runtime,
        run_id,
        vec![
            json!({
                "schemaVersion": 1,
                "event_type": "run.started",
                "event_id": "evt-run",
                "ts": "2026-05-08T04:12:20Z",
                "run_id": run_id,
                "body_kind": "run_started"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step",
                "ts": "2026-05-08T04:12:21Z",
                "run_id": run_id,
                "parent_event_id": "evt-run",
                "body_kind": "step_started",
                "step_id": "implement"
            }),
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
            }),
        ],
    );
    stderr_ref
}

fn seed_v2_audit_events(
    runtime: &OrbitRuntime,
    run_id: &str,
    events: impl IntoIterator<Item = Value>,
) {
    let workspace_id = runtime.workspace_id().expect("workspace id");
    for (index, mut event) in events.into_iter().enumerate() {
        let object = event.as_object_mut().expect("event object");
        object
            .entry("schemaVersion".to_string())
            .or_insert_with(|| json!(1));
        object
            .entry("run_id".to_string())
            .or_insert_with(|| json!(run_id));
        object
            .entry("agent_identity".to_string())
            .or_insert_with(|| json!("system"));
        object.entry("ts".to_string()).or_insert_with(|| {
            json!(format!(
                "2026-05-08T04:{:02}:{:02}Z",
                (index / 60) % 60,
                index % 60
            ))
        });

        let ts = event
            .get("ts")
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .expect("event ts");
        runtime
            .insert_v2_audit_event(&V2AuditEventInsertParams {
                workspace_id: workspace_id.clone(),
                event_id: event["event_id"].as_str().expect("event id").to_string(),
                source: "v2_envelope".to_string(),
                schema_version: event["schemaVersion"]
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(1),
                event_type: event["event_type"]
                    .as_str()
                    .expect("event type")
                    .to_string(),
                ts,
                run_id: event["run_id"].as_str().expect("run id").to_string(),
                agent_identity: event["agent_identity"]
                    .as_str()
                    .expect("agent identity")
                    .to_string(),
                parent_event_id: event
                    .get("parent_event_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                workspace_path: None,
                payload_json: event.to_string(),
            })
            .expect("insert v2 audit event");
    }
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
async fn list_run_events_streams_small_page_from_oversized_fixture() {
    // The persisted audit table can contain many rows for a run. The endpoint
    // must fill a small page from the head of the result set without scanning
    // every event first.
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-events-oversize";
    seed_v2_audit_events(
        &runtime,
        run_id,
        (0..10usize).map(|index| {
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": format!("evt-{index}"),
                "run_id": run_id,
                "body_kind": "step_started"
            })
        }),
    );

    let response = request_dashboard_run_events_query(runtime, run_id, "?limit=5").await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let events = payload.as_array().expect("events array");
    assert_eq!(events.len(), 5);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["event_id"], format!("evt-{index}"));
        assert_eq!(event["body_kind"], "step_started");
    }
}

#[tokio::test]
async fn list_run_events_returns_payload_too_large_when_scan_budget_exceeded() {
    // Fill the table with valid events whose `body_kind` does NOT match the
    // requested filter, forcing the endpoint to walk past the row budget
    // without ever filling the page.
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-events-budget";
    seed_v2_audit_events(
        &runtime,
        run_id,
        (0..=super::super::runs::RUN_EVENTS_MAX_SCAN_LINES).map(|index| {
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": format!("evt-budget-{index}"),
                "run_id": run_id,
                "body_kind": "step_started"
            })
        }),
    );

    let response =
        request_dashboard_run_events_query(runtime, run_id, "?kind=does_not_match").await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let payload = body_json(response).await;
    let error = payload["error"].as_str().expect("error message");
    assert!(
        error.contains("bounded scan budget"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn list_run_events_kind_filter_still_works_with_streaming() {
    // Confirms AC3: streaming preserves kind filtering correctness.
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-events-kind";
    seed_v2_audit_events(
        &runtime,
        run_id,
        vec![
            json!({
                "schemaVersion": 1,
                "event_type": "run.started",
                "event_id": "evt-run",
                "run_id": run_id,
                "body_kind": "run_started"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-a",
                "run_id": run_id,
                "body_kind": "step_started"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-b",
                "run_id": run_id,
                "body_kind": "step_started"
            }),
        ],
    );

    let response = request_dashboard_run_events_query(runtime, run_id, "?kind=step_started").await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let events = payload.as_array().expect("events array");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_id"], "evt-step-a");
    assert_eq!(events[1]["event_id"], "evt-step-b");
}

#[tokio::test]
async fn list_run_events_accepts_valid_run_id() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-1";
    seed_v2_audit_events(
        &runtime,
        run_id,
        vec![json!({
            "schemaVersion": 1,
            "event_type": "step.started",
            "event_id": "evt-step-started",
            "run_id": run_id,
            "body_kind": "step_started"
        })],
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
async fn resume_job_run_endpoint_rejects_non_terminal_run_with_guard_reason() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let source = seed_run(
        &runtime,
        "jrun-web-resume-pending",
        "web_resume_pending",
        JobRunState::Pending,
    );

    let response = request_resume(runtime, &source.run_id, Some("http://localhost:3000")).await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("is pending"))
    );
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("interrupted, failed, or timed-out"))
    );
}

#[tokio::test]
async fn resume_job_run_endpoint_returns_not_found_for_unknown_run() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_resume(
        runtime,
        "jrun-web-resume-missing",
        Some("http://localhost:3000"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let payload = body_json(response).await;
    assert_eq!(payload["error"], "run not found: jrun-web-resume-missing");
}

#[test]
fn run_detail_uses_v2_audit_steps_when_step_bundle_is_empty() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-web-audit-step";
    seed_v2_audit_events(
        &runtime,
        run_id,
        vec![
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-started",
                "ts": "2026-04-28T00:00:01Z",
                "run_id": run_id,
                "agent_identity": "system",
                "body_kind": "step_started",
                "step_id": "build"
            }),
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
            }),
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
        resolved_crew: None,
        crew_model: None,
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

#[test]
fn run_detail_exposes_the_provider_pid_and_liveness_for_an_open_agent_step() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run_id = "jrun-web-provider-pid";
    seed_v2_audit_events(
        &runtime,
        run_id,
        vec![
            json!({
                "schemaVersion": 1,
                "event_type": "step.started",
                "event_id": "evt-step-started",
                "ts": "2026-07-27T02:41:01Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "body_kind": "step_started",
                "step_id": "agent_implement"
            }),
            json!({
                "schemaVersion": 1,
                "event_type": "cli.invocation.process",
                "event_id": "evt-pid",
                "ts": "2026-07-27T02:41:02Z",
                "run_id": run_id,
                "agent_identity": "codex",
                "parent_event_id": "evt-step-started",
                "body_kind": "cli_invocation_process",
                "provider": "codex",
                // Unreachable PID: the probe must resolve it as gone rather
                // than reporting a live child that does not exist.
                "pid": u32::MAX - 1,
                "pid_start_time": "ps-lstart-utc-v1:seeded"
            }),
        ],
    );
    let scheduled_at = chrono::DateTime::parse_from_rfc3339("2026-07-27T02:41:00Z")
        .expect("parse scheduled")
        .with_timezone(&Utc);
    let run = orbit_core::JobRun {
        run_id: run_id.to_string(),
        job_id: "task_pr_pipeline".to_string(),
        attempt: 1,
        state: JobRunState::Running,
        scheduled_at,
        started_at: Some(scheduled_at),
        finished_at: None,
        duration_ms: None,
        created_at: scheduled_at,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    };

    let detail = job_run_detail_to_json(&runtime, &run);
    let processes = detail["provider_processes"]
        .as_array()
        .expect("provider_processes array");

    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0]["pid"], u32::MAX - 1);
    assert_eq!(processes[0]["provider"], "codex");
    assert_eq!(processes[0]["step_id"], "agent_implement");
    assert_eq!(processes[0]["finished"], false);
    assert_eq!(processes[0]["exit_code"], Value::Null);
    if cfg!(unix) {
        assert_eq!(processes[0]["liveness"], "exited");
    } else {
        assert_eq!(processes[0]["liveness"], "unknown");
    }
}

async fn request_ship(runtime: OrbitRuntime, body: Option<Value>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/workflows/ship")
        .header(header::ORIGIN, "http://localhost:3000");
    let body = match body {
        Some(json) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response")
}

#[tokio::test]
async fn ship_endpoint_rejects_unknown_mode() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_ship(runtime, Some(json!({ "mode": "yolo" }))).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("unknown ship mode"))
    );
}

async fn request_ship_global(
    state: crate::state::DashboardState,
    uri: &str,
    body: Value,
) -> Response {
    router()
        .with_state(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::ORIGIN, "http://localhost:7878")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response")
}

/// ORB-10008: an unknown `?workspace=` on the ship endpoint is a clean 404
/// JSON rejection from the workspace extractor, not a 500.
#[tokio::test]
async fn ship_endpoint_rejects_unknown_workspace_with_404_json() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let state = crate::state::DashboardState::single(Arc::new(runtime));

    let response = request_ship_global(
        state,
        "/workflows/ship?workspace=ghost",
        json!({ "mode": "pr" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|m| m.contains("unknown workspace: ghost"))
    );
}

/// ORB-10444: Ship is a write against a live pipeline, so a second click while
/// the task already has a run in flight must not create a duplicate run. The
/// in-flight run is seeded as `pending` with no owner pid, which the run-owner
/// reconciler leaves alone inside its unclaimed grace window — so the guard,
/// not a reconciliation race, is what this asserts.
///
/// Explicit task ids are validated before the guard (shared `submit_ship_run`
/// path), so the fixture seeds a real backlog task rather than a synthetic id.
/// Refusal still happens before job submit, so no pipeline worker is spawned.
#[tokio::test]
async fn ship_endpoint_refuses_second_dispatch_while_task_run_is_in_flight() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "in-flight ship guard fixture".to_string(),
            description: "seeded so explicit ship selection validates".to_string(),
            status: Some(TaskStatus::Backlog),
            ..TaskAddParams::default()
        })
        .expect("seed ship fixture task")
        .id;
    let mut in_flight = seed_run(
        &runtime,
        "jrun-in-flight",
        "task_auto_pipeline",
        JobRunState::Pending,
    );
    in_flight.input = Some(json!({ "mode": "local", "task_ids": [task_id] }));
    write_seeded_run(&runtime, &in_flight);

    let response = request_ship(
        runtime.clone(),
        Some(json!({ "task_ids": [task_id], "mode": "local" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_json(response).await;
    assert_eq!(payload["code"].as_str(), Some("ship_run_in_flight"));
    assert_eq!(payload["run_id"].as_str(), Some("jrun-in-flight"));
    assert_eq!(payload["task_id"].as_str(), Some(task_id.as_str()));

    let runs = runtime
        .list_job_runs(JobRunListParams::default())
        .expect("list runs");
    assert_eq!(
        runs.len(),
        1,
        "the rejected second click must not persist another run: {runs:?}"
    );
}

#[tokio::test]
async fn ship_endpoint_rejects_duplicate_task_ids() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");

    let response = request_ship(
        runtime,
        Some(json!({ "task_ids": ["T12345678-123456", "T12345678-123456"] })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("duplicate task id"))
    );
}
