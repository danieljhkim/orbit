//! Endpoint tests for `GET /audit` (listing, paging, filters, error paths).
//!
//! ORB-10008: the audit surface previously had no handler-level coverage; these
//! tests drive the real router with seeded SQLite audit events.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use orbit_core::{AuditEventInsertParams, AuditEventStatus, OrbitRuntime};
use orbit_types::tool::{McpCapability, McpTransport};
use serde_json::Value;
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

fn seed_audit_event(
    runtime: &OrbitRuntime,
    execution_id: &str,
    tool_name: &str,
    status: AuditEventStatus,
    role: &str,
    error_message: Option<&str>,
) {
    runtime
        .record_audit_event(&AuditEventInsertParams {
            execution_id: execution_id.to_string(),
            command: "task".to_string(),
            subcommand: Some("update".to_string()),
            tool_name: Some(tool_name.to_string()),
            target_type: Some("task".to_string()),
            target_id: Some("T00000000-000000".to_string()),
            role: role.to_string(),
            status,
            exit_code: match status {
                AuditEventStatus::Success => 0,
                _ => 1,
            },
            duration_ms: 5,
            working_directory: "/tmp/fixture".to_string(),
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: error_message.map(str::to_string),
            host: None,
            pid: std::process::id(),
            session_id: None,
            workspace_id: Some("ws-orbit".to_string()),
            caller_machine_id: Some("hm-caller".to_string()),
            caller_host_id: Some("caller.local".to_string()),
            process_machine_id: Some("hm-process".to_string()),
            process_host_id: Some("process.local".to_string()),
            transport: Some(McpTransport::Local),
            effective_capabilities: [McpCapability::Agent, McpCapability::Runner]
                .into_iter()
                .collect(),
            origin_session_id: Some("mcp-session".to_string()),
            mcp_call_id: Some("mcall".to_string()),
            lease_id: Some("lease".to_string()),
            task_id: None,
            job_run_id: Some("jrun".to_string()),
            activity_id: None,
            step_index: None,
        })
        .expect("seed audit event");
}

async fn request_audit(runtime: OrbitRuntime, uri: &str) -> axum::response::Response {
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

fn execution_ids(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .map(|row| row["execution_id"].as_str().expect("execution_id"))
        .collect()
}

#[tokio::test]
async fn audit_lists_seeded_events_newest_first_with_projected_fields() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_audit_event(
        &runtime,
        "exec-1",
        "orbit.task.update",
        AuditEventStatus::Success,
        "editor",
        None,
    );
    seed_audit_event(
        &runtime,
        "exec-2",
        "orbit.task.add",
        AuditEventStatus::Failure,
        "editor",
        Some("boom"),
    );

    let response = request_audit(runtime, "/audit").await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("audit array");
    assert_eq!(rows.len(), 2);
    // SQLite lists `ORDER BY id DESC`: the most recently inserted event first.
    assert_eq!(execution_ids(rows), vec!["exec-2", "exec-1"]);
    let newest = &rows[0];
    assert_eq!(newest["tool_name"], "orbit.task.add");
    assert_eq!(newest["status"], "failure");
    assert_eq!(newest["role"], "editor");
    assert_eq!(newest["error_message"], "boom");
    assert_eq!(newest["workspace_id"], "ws-orbit");
    assert_eq!(newest["caller_machine_id"], "hm-caller");
    assert_eq!(newest["caller_host_id"], "caller.local");
    assert_eq!(newest["process_machine_id"], "hm-process");
    assert_eq!(newest["process_host_id"], "process.local");
    assert_eq!(newest["transport"], "local");
    assert_eq!(
        newest["effective_capabilities"],
        serde_json::json!(["agent", "runner"])
    );
    assert_eq!(newest["origin_session_id"], "mcp-session");
    assert_eq!(newest["mcp_call_id"], "mcall");
    assert_eq!(newest["lease_id"], "lease");
    assert!(
        newest["timestamp"]
            .as_str()
            .is_some_and(|ts| { chrono::DateTime::parse_from_rfc3339(ts).is_ok() })
    );
}

#[tokio::test]
async fn audit_filters_all_trusted_mcp_provenance_fields() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_audit_event(
        &runtime,
        "exec-trusted",
        "orbit.task.list",
        AuditEventStatus::Success,
        "unverified",
        None,
    );

    let response = request_audit(
        runtime,
        concat!(
            "/audit?workspace_id=ws-orbit&caller_machine=hm-caller",
            "&process_machine=hm-process&transport=local&capability=runner",
            "&origin_session=mcp-session&mcp_call=mcall&job_run_id=jrun&lease=lease"
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("trusted provenance filtered rows");
    assert_eq!(execution_ids(rows), vec!["exec-trusted"]);
}

#[tokio::test]
async fn audit_limit_and_offset_page_through_results_without_overlap() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    for index in 0..5 {
        seed_audit_event(
            &runtime,
            &format!("exec-{index}"),
            "orbit.task.update",
            AuditEventStatus::Success,
            "editor",
            None,
        );
    }

    let response = request_audit(runtime.clone(), "/audit?limit=3").await;
    assert_eq!(response.status(), StatusCode::OK);
    let first = body_json(response).await;
    let first_rows = first.as_array().expect("first page");
    assert_eq!(first_rows.len(), 3);

    let response = request_audit(runtime.clone(), "/audit?limit=3&offset=3").await;
    assert_eq!(response.status(), StatusCode::OK);
    let second = body_json(response).await;
    let second_rows = second.as_array().expect("second page");
    assert_eq!(second_rows.len(), 2);

    let mut all: Vec<String> = execution_ids(first_rows)
        .into_iter()
        .chain(execution_ids(second_rows))
        .map(str::to_string)
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 5, "pages must be disjoint and cover every event");

    // Offset past the end degrades to an empty page, not an error.
    let response = request_audit(runtime, "/audit?offset=50").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body, Value::Array(Vec::new()));
}

#[tokio::test]
async fn audit_filters_by_tool_status_run_id_alias_and_text_query() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_audit_event(
        &runtime,
        "exec-ok",
        "orbit.task.update",
        AuditEventStatus::Success,
        "editor",
        None,
    );
    seed_audit_event(
        &runtime,
        "exec-denied",
        "orbit.policy.check",
        AuditEventStatus::Denied,
        "admin",
        Some("blocked by policy needle"),
    );

    let response = request_audit(runtime.clone(), "/audit?tool=orbit.policy.check").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("tool-filtered array");
    assert_eq!(execution_ids(rows), vec!["exec-denied"]);

    let response = request_audit(runtime.clone(), "/audit?status=denied").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("status-filtered array");
    assert_eq!(execution_ids(rows), vec!["exec-denied"]);

    // `run_id` is the backward-compat alias of `execution_id` (T20260427-26).
    let response = request_audit(runtime.clone(), "/audit?run_id=exec-ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("run_id-filtered array");
    assert_eq!(execution_ids(rows), vec!["exec-ok"]);

    // When both are supplied, `execution_id` wins.
    let response = request_audit(
        runtime.clone(),
        "/audit?execution_id=exec-denied&run_id=exec-ok",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("execution_id-precedence array");
    assert_eq!(execution_ids(rows), vec!["exec-denied"]);

    let response = request_audit(runtime, "/audit?q=needle").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("text-filtered array");
    assert_eq!(execution_ids(rows), vec!["exec-denied"]);
}

#[tokio::test]
async fn audit_rejects_malformed_since_and_status_with_json_400() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_audit(runtime.clone(), "/audit?since=not-a-time").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(body["error"].as_str().is_some_and(|m| !m.is_empty()));

    let response = request_audit(runtime, "/audit?status=bogus").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("unknown audit event status"))
    );
}

#[tokio::test]
async fn audit_rejects_non_numeric_limit_without_500() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request_audit(runtime, "/audit?limit=lots").await;

    // Axum's Query extractor rejects before the handler runs: a clean client
    // error, never a 500/panic.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// ORB-10871: the header summary reports the raw failed-event count and the
/// grouped incident count as two distinct fields over the same window, so a
/// repeated burst can no longer be read as many independent failures.
#[tokio::test]
async fn audit_summary_separates_raw_failed_events_from_grouped_incidents() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    for index in 0..12 {
        seed_audit_event(
            &runtime,
            &format!("exec-burst-{index}"),
            "surface.alpha",
            AuditEventStatus::Failure,
            "actor-one",
            Some(&format!("could not remove /work/dir-{index}/file.txt")),
        );
    }
    seed_audit_event(
        &runtime,
        "exec-denied-one",
        "surface.beta",
        AuditEventStatus::Denied,
        "actor-one",
        Some("policy denied: write outside the allowed scope"),
    );

    let response = request_audit(runtime, "/audit/summary?since=24h").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    assert_eq!(body["window"], "24h", "both counts state their window");
    assert_eq!(body["events"].as_u64(), Some(13), "all-events denominator");
    assert_eq!(
        body["failed_events"].as_u64(),
        Some(13),
        "raw failed rows stay counted in full"
    );
    assert_eq!(
        body["failure_incidents"].as_u64(),
        Some(2),
        "the burst collapses to one incident; the denial stays its own"
    );
    assert_eq!(
        body["failure_incidents_by_class"]["denied"].as_u64(),
        Some(1)
    );
    assert_eq!(
        body["failed_events_by_class"]["unexpected"].as_u64(),
        Some(12)
    );
}
