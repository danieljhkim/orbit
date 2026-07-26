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
use orbit_core::runtime::WorkspaceRuntimeBinding;
use orbit_core::{JobRunState, OrbitRuntime, ShipMode, V2AuditEventInsertParams};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::super::runs::*;
use super::test_support::{
    body_json, seed_run, write_replay_job, write_replay_job_under, write_seeded_run,
};

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

async fn request_replay(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/runs/{run_id}/replay"));
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
        .with_state(crate::state::DashboardState::single(Arc::new(
            runtime.clone(),
        )))
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

/// [ORB-10470] Resume is a submission, not an execution: the response carries
/// the new run id and its lineage as soon as the run is durable, and the
/// resumed pipeline runs in a detached worker rather than on the request
/// thread (F2026-07-122 defect 3).
#[tokio::test]
async fn resume_job_run_endpoint_submits_a_linked_run_without_executing_it_in_request() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "web_resume_success");
    let mut source = seed_run(
        &runtime,
        "jrun-web-resume-failed",
        "web_resume_success",
        JobRunState::Failed,
    );
    source.input = Some(json!({ "seconds": 0 }));
    write_seeded_run(&runtime, &source);

    let response = request_resume(
        runtime.clone(),
        &source.run_id,
        Some("http://localhost:3000"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let new_run_id = payload["run_id"].as_str().expect("new run id");
    assert_ne!(new_run_id, source.run_id);
    assert_eq!(payload["workflow"], "resume");
    assert_eq!(payload["job_id"], "web_resume_success");
    assert!(
        matches!(payload["state"].as_str(), Some("submitted" | "queued")),
        "resume returns a submission state, not a terminal one: {payload}",
    );
    assert_eq!(
        payload["retry_source_run_id"].as_str(),
        Some(source.run_id.as_str())
    );
    let stored = runtime.show_job_run(new_run_id).expect("show resumed run");
    assert_eq!(
        stored.retry_source_run_id.as_deref(),
        Some(source.run_id.as_str())
    );
    assert_eq!(stored.attempt, source.attempt + 1);

    // Run listing answers while the resumed run is outstanding.
    let list_response = router()
        .with_state(crate::state::DashboardState::single(Arc::new(
            runtime.clone(),
        )))
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
fn independent_review_run_detail_projects_crew_and_exact_head_lineage() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let scheduled_at = Utc::now();
    let run = orbit_core::JobRun {
        run_id: "jrun-independent-review".to_string(),
        job_id: "task_review_pipeline".to_string(),
        attempt: 1,
        state: JobRunState::Success,
        scheduled_at,
        started_at: Some(scheduled_at),
        finished_at: Some(scheduled_at),
        duration_ms: Some(1),
        created_at: scheduled_at,
        pid: None,
        pid_start_time: None,
        input: Some(json!({
            "task_ids": ["ORB-10266"],
            "workspace_path": "/tmp/review-worktree",
            "crew": "opus",
            "parent_run_id": "jrun-parent",
            "candidate_head": "orbit/ORB-10266-branch",
            "candidate_head_sha": "abc123",
            "pr_number": "633",
        })),
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: Some("opus".to_string()),
        crew_model: Some("opus".to_string()),
        steps: Vec::new(),
    };

    let detail = job_run_detail_to_json(&runtime, &run);
    assert_eq!(detail["run"]["job_id"], "task_review_pipeline");
    assert_eq!(detail["run"]["resolved_crew"], "opus");
    let lineage = &detail["run"]["review_lineage"];
    assert_eq!(lineage["parent_run_id"], "jrun-parent");
    assert_eq!(lineage["task_ids"], json!(["ORB-10266"]));
    assert_eq!(lineage["workspace_path"], "/tmp/review-worktree");
    assert_eq!(lineage["candidate_head_sha"], "abc123");
    assert_eq!(lineage["pr_number"], "633");
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

fn review_ship_runtime() -> (tempfile::TempDir, OrbitRuntime, String) {
    let root = tempfile::tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let orbit_root = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&orbit_root).expect("create orbit root");
    std::fs::write(
        orbit_root.join("config.toml"),
        r#"
[workflow]
base_branch = "main"
default_crew = "sol"

[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
backend = "cli"

[crews.opus]
model = "opus"
provider = "claude"
backend = "cli"
"#,
    )
    .expect("write config");
    let runtime = OrbitRuntime::from_roots(&global_root, &orbit_root).expect("build runtime");
    seed_review_ship_assets(&runtime);
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "review ship endpoint fixture".to_string(),
            description: "persist explicit review controls".to_string(),
            plan: "submit only".to_string(),
            status: Some(orbit_core::TaskStatus::Backlog),
            crew: Some("sol".to_string()),
            ..TaskAddParams::default()
        })
        .expect("add review fixture task")
        .id;
    (root, runtime, task_id)
}

fn seed_review_ship_assets(runtime: &OrbitRuntime) {
    let jobs = runtime.global_root().join("resources/jobs");
    let activities = runtime.global_root().join("resources/activities");
    std::fs::create_dir_all(&jobs).expect("create jobs");
    std::fs::create_dir_all(&activities).expect("create activities");

    let forwarding_stub = |name: &str| {
        format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  default_input:
    review: false
    review_crew: null
  steps:
    - id: nap
      default_input:
        seconds: 0
        review: "{{{{ input.review }}}}"
        review_crew: "{{{{ input.review_crew }}}}"
      spec:
        type: deterministic
        action: sleep
        config: {{}}
"#
        )
    };
    std::fs::write(
        jobs.join("task_auto_pipeline.yaml"),
        forwarding_stub("task_auto_pipeline"),
    )
    .expect("write auto job");
    std::fs::write(
        jobs.join("task_gate_pipeline.yaml"),
        forwarding_stub("task_gate_pipeline"),
    )
    .expect("write gate job");
    std::fs::write(
        jobs.join("task_pr_pipeline.yaml"),
        r#"schemaVersion: 2
kind: Job
metadata:
  name: task_pr_pipeline
spec:
  state: enabled
  kind: workflow
  steps:
    - id: push
      spec: { type: deterministic, action: sleep, config: {} }
    - id: pr_open
      spec: { type: deterministic, action: sleep, config: {} }
    - id: promote_tasks
      spec: { type: deterministic, action: sleep, config: {} }
    - id: independent_review
      when: "{{ input.review }} == true && {{ steps.commit.output.skipped_no_diff_expected }} != true"
      target: activity:invoke_and_wait
      default_input:
        job_name: task_review_pipeline
        run_input:
          task_ids: "{{ input.task_ids }}"
          workspace_path: "{{ input.workspace_path }}"
          crew: "{{ input.review_crew }}"
          parent_run_id: "{{ input.parent_run_id }}"
          candidate_head: "{{ input.candidate_head }}"
          candidate_head_sha: "{{ input.candidate_head_sha }}"
          pr_number: "{{ input.pr_number }}"
        dedupe_run_input_field: parent_run_id
    - id: require_independent_review_success
      target: activity:pipeline_success_guard
      default_input: { result: "{{ steps.independent_review.output }}" }
"#,
    )
    .expect("write PR job");
    std::fs::write(
        jobs.join("task_review_pipeline.yaml"),
        r#"schemaVersion: 2
kind: Job
metadata:
  name: task_review_pipeline
spec:
  state: enabled
  kind: workflow
  steps:
    - id: independent_review
      target: activity:agent_review
      default_input:
        task_ids: "{{ input.task_ids }}"
        workspace_path: "{{ input.workspace_path }}"
        crew: "{{ input.crew }}"
        parent_run_id: "{{ input.parent_run_id }}"
        candidate_head: "{{ input.candidate_head }}"
        candidate_head_sha: "{{ input.candidate_head_sha }}"
        pr_number: "{{ input.pr_number }}"
    - id: guard
      target: activity:independent_review_guard
"#,
    )
    .expect("write review job");
    std::fs::write(
        activities.join("agent_review.yaml"),
        r#"schemaVersion: 2
kind: Activity
metadata:
  name: agent_review
spec:
  type: agent_loop
  description: fixture
  output_schema_json:
    type: object
    required: [verdict, reviewed_head_sha]
  instruction: fixture
  require_response_envelope: true
"#,
    )
    .expect("write review activity");
    std::fs::write(
        activities.join("independent_review_guard.yaml"),
        r#"schemaVersion: 2
kind: Activity
metadata:
  name: independent_review_guard
spec:
  type: deterministic
  description: fixture
  action: independent_review_guard
  config: {}
"#,
    )
    .expect("write guard activity");
    for name in ["invoke_and_wait", "pipeline_success_guard"] {
        std::fs::write(
            activities.join(format!("{name}.yaml")),
            format!(
                r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: deterministic
  description: fixture
  action: {name}
  config: {{}}
"#
            ),
        )
        .expect("write orchestration activity");
    }
}

#[tokio::test]
async fn ship_endpoint_submits_task_auto_pipeline_run() {
    let (_root, runtime, task_id) = review_ship_runtime();

    let response = request_ship(
        runtime.clone(),
        Some(json!({
            "task_ids": [task_id],
            "mode": "pr",
            "review": true,
            "review_crew": "opus",
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["workflow"].as_str(), Some("ship"));
    assert_eq!(payload["job_id"].as_str(), Some("task_auto_pipeline"));
    assert!(matches!(
        payload["state"].as_str(),
        Some("queued" | "submitted")
    ));
    let run_id = payload["run_id"].as_str().expect("run id");
    let stored = runtime.show_job_run(run_id).expect("stored ship run");
    assert_eq!(stored.job_id, "task_auto_pipeline");
    assert_eq!(
        stored.input,
        Some(json!({
            "mode": "pr",
            "base_branch": "main",
            "task_ids": [task_id],
            "review": true,
            "review_crew": "opus",
        }))
    );
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

#[tokio::test]
async fn ship_endpoint_rejects_enabled_review_without_explicit_crew() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_ship(
        runtime,
        Some(json!({ "task_ids": ["ORB-99999"], "review": true })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("non-blank explicit review crew"))
    );
}

/// Create an on-disk workspace under `base/<name>`, so global mode can build
/// its runtime lazily via `from_roots`. Returns `(orbit_dir, repo_root)`. The
/// `task_auto_pipeline` job asset itself is a *default* job resolved from the
/// global orbit root — seed it there once via `write_replay_job_under`.
fn seed_ship_workspace(
    base: &std::path::Path,
    name: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo_root = base.join(name);
    let orbit_dir = repo_root.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create .orbit");
    std::fs::write(orbit_dir.join("config.toml"), "").expect("write config");
    std::fs::write(
        orbit_dir.join("config.yaml"),
        format!("schema_version: 1\nworkspace_id: ws_{name}\n"),
    )
    .expect("write workspace identity");
    (orbit_dir, repo_root)
}

fn ship_workspace_entry(
    id: &str,
    repo_root: std::path::PathBuf,
    orbit_dir: std::path::PathBuf,
) -> crate::state::WsEntry {
    crate::state::WsEntry {
        id: id.to_string(),
        name: id.to_string(),
        binding: Some(WorkspaceRuntimeBinding {
            workspace_id: format!("ws_{id}"),
            repo_root: repo_root.clone(),
            ship_mode: ShipMode::Local,
        }),
        repo_root,
        orbit_dir,
        active: true,
    }
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

/// ORB-10008: `POST /workflows/ship?workspace=<id>` in aggregate (global)
/// mode submits the run into the selected workspace only. Drives the real
/// submission path (job asset load, run insert, worker spawn) over on-disk
/// temp workspaces; the job asset is the stub sleep workflow, so no git or
/// agent machinery runs.
#[tokio::test]
async fn ship_endpoint_in_global_mode_targets_selected_workspace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");
    write_replay_job_under(&global_root, "task_auto_pipeline");
    let (alpha_orbit, alpha_repo) = seed_ship_workspace(tmp.path(), "alpha");
    let (beta_orbit, beta_repo) = seed_ship_workspace(tmp.path(), "beta");
    let entries = vec![
        ship_workspace_entry("alpha", alpha_repo, alpha_orbit.clone()),
        ship_workspace_entry("beta", beta_repo, beta_orbit.clone()),
    ];
    let state = crate::state::DashboardState::global(
        global_root.clone(),
        entries,
        Some("alpha".to_string()),
    );

    // Exercise the non-default `local` ship mode against the non-default
    // workspace so both selection and mode parsing are load-bearing.
    let response = request_ship_global(
        state,
        "/workflows/ship?workspace=beta",
        json!({ "mode": "local" }),
    )
    .await;

    let status = response.status();
    let payload = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {payload}");
    assert_eq!(payload["workflow"].as_str(), Some("ship"));
    assert_eq!(payload["job_id"].as_str(), Some("task_auto_pipeline"));
    assert!(matches!(
        payload["state"].as_str(),
        Some("queued" | "submitted")
    ));
    let run_id = payload["run_id"].as_str().expect("run id");

    // The run is persisted in beta...
    let beta_runtime =
        OrbitRuntime::from_roots(&global_root, &beta_orbit).expect("reopen beta workspace");
    let stored = beta_runtime.show_job_run(run_id).expect("stored ship run");
    assert_eq!(stored.job_id, "task_auto_pipeline");
    // ...and nowhere else.
    let alpha_runtime =
        OrbitRuntime::from_roots(&global_root, &alpha_orbit).expect("reopen alpha workspace");
    assert!(alpha_runtime.show_job_run(run_id).is_err());
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

#[tokio::test]
async fn ship_endpoint_review_false_preserves_implementation_only_input() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");

    let response = request_ship(
        runtime.clone(),
        Some(json!({
            "mode": "pr",
            "review": false,
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    let run_id = payload["run_id"].as_str().expect("run id");
    let run = runtime
        .list_job_runs(JobRunListParams::default())
        .expect("list submitted runs")
        .into_iter()
        .find(|run| run.run_id == run_id)
        .expect("submitted run exists");
    let input = run.input.expect("persisted run input");
    assert_eq!(input["mode"], "pr");
    assert!(input.get("review").is_none());
    assert!(input.get("review_crew").is_none());
}

/// ORB-10444: the dashboard's one-click Ship posts nothing but the task id —
/// no crew and no mode. The dispatch must carry that task id through, resolve
/// the mode from the selected workspace's own binding (`local` here, not the
/// endpoint's historical `pr` default), and leave crew resolution to the
/// pipeline, which reads the task's own record.
#[tokio::test]
async fn ship_endpoint_without_overrides_uses_task_id_and_workspace_ship_mode() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");
    write_replay_job_under(&global_root, "task_auto_pipeline");
    let (beta_orbit, beta_repo) = seed_ship_workspace(tmp.path(), "beta");
    let state = crate::state::DashboardState::global(
        global_root.clone(),
        vec![ship_workspace_entry("beta", beta_repo, beta_orbit.clone())],
        Some("beta".to_string()),
    );

    // Exactly the body the Ship button sends: the task id and nothing else.
    let response = request_ship_global(
        state,
        "/workflows/ship",
        json!({ "task_ids": ["ORB-10444"] }),
    )
    .await;

    let status = response.status();
    let payload = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {payload}");
    let run_id = payload["run_id"].as_str().expect("run id");

    let beta_runtime =
        OrbitRuntime::from_roots(&global_root, &beta_orbit).expect("reopen beta workspace");
    let input = beta_runtime
        .show_job_run(run_id)
        .expect("stored ship run")
        .input
        .expect("persisted run input");
    assert_eq!(input["task_ids"], json!(["ORB-10444"]));
    // The workspace entry is bound to `ShipMode::Local`, so an omitted `mode`
    // resolves to `local` rather than the endpoint's legacy `pr` fallback.
    assert_eq!(input["mode"], "local");
    assert!(
        input.get("crew").is_none() && input.get("review_crew").is_none(),
        "one-click Ship must not send a crew override: {input}"
    );
    assert!(
        input.get("review").is_none(),
        "one-click Ship must not enable the review step: {input}"
    );
}

/// ORB-10444: Ship is a write against a live pipeline, so a second click while
/// the task already has a run in flight must not create a duplicate run. The
/// in-flight run is seeded as `pending` with no owner pid, which the run-owner
/// reconciler leaves alone inside its unclaimed grace window — so the guard,
/// not a reconciliation race, is what this asserts.
#[tokio::test]
async fn ship_endpoint_refuses_second_dispatch_while_task_run_is_in_flight() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");
    let mut in_flight = seed_run(
        &runtime,
        "jrun-in-flight",
        "task_auto_pipeline",
        JobRunState::Pending,
    );
    in_flight.input = Some(json!({ "mode": "local", "task_ids": ["ORB-10444"] }));
    write_seeded_run(&runtime, &in_flight);

    let response = request_ship(
        runtime.clone(),
        Some(json!({ "task_ids": ["ORB-10444"], "mode": "local" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_json(response).await;
    assert_eq!(payload["code"].as_str(), Some("ship_run_in_flight"));
    assert_eq!(payload["run_id"].as_str(), Some("jrun-in-flight"));
    assert_eq!(payload["task_id"].as_str(), Some("ORB-10444"));

    let runs = runtime
        .list_job_runs(JobRunListParams::default())
        .expect("list runs");
    assert_eq!(
        runs.len(),
        1,
        "the rejected second click must not persist another run: {runs:?}"
    );
}

/// The in-flight guard keys on the task id, so a *different* task is still
/// shippable while one run is in flight, and auto (no task ids) mode — which
/// has nothing to key on — is untouched.
#[tokio::test]
async fn ship_endpoint_in_flight_guard_is_scoped_to_the_shipped_task() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");
    let mut in_flight = seed_run(
        &runtime,
        "jrun-other-task",
        "task_auto_pipeline",
        JobRunState::Pending,
    );
    in_flight.input = Some(json!({ "mode": "local", "task_ids": ["ORB-10001"] }));
    write_seeded_run(&runtime, &in_flight);

    let response = request_ship(
        runtime.clone(),
        Some(json!({ "task_ids": ["ORB-10444"], "mode": "local" })),
    )
    .await;

    let status = response.status();
    let payload = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {payload}");
}

/// A terminal run holding the same task id is history, not contention: the
/// guard must not wedge re-shipping a task whose previous run already finished.
#[tokio::test]
async fn ship_endpoint_allows_dispatch_after_the_previous_run_is_terminal() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    write_replay_job(&runtime, "task_auto_pipeline");
    let mut finished = seed_run(
        &runtime,
        "jrun-finished",
        "task_auto_pipeline",
        JobRunState::Success,
    );
    finished.input = Some(json!({ "mode": "local", "task_ids": ["ORB-10444"] }));
    write_seeded_run(&runtime, &finished);

    let response = request_ship(
        runtime.clone(),
        Some(json!({ "task_ids": ["ORB-10444"], "mode": "local" })),
    )
    .await;

    let status = response.status();
    let payload = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {payload}");
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
