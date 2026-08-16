// ORB-00337: window-aware scoreboard endpoint contract.
//
// Asserts the HTTP surface for `?window=` honors the scoreboard windowing
// behavior added in orbit-store / orbit-core:
// - missing param defaults to lifetime (`window: "all"`)
// - `?window=1h` round-trips into the serialized payload + populates
//   `window_since`
// - unknown values produce HTTP 400 (not a 500)
// - schema_version is the post-retirement v8 value with its separately-versioned
//   managed-execution orchestration section

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use orbit_core::OrbitRuntime;
use tower::ServiceExt;

use super::super::*;
use super::test_support::body_json;

async fn get_scoreboard(runtime: OrbitRuntime, query: Option<&str>) -> axum::response::Response {
    let uri = match query {
        Some(q) => format!("/scoreboard?{q}"),
        None => "/scoreboard".to_string(),
    };
    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("response")
}

#[tokio::test]
async fn scoreboard_default_returns_lifetime_window_and_v8_schema() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["schema_version"].as_u64(), Some(8));
    assert_eq!(body["window"].as_str(), Some("all"));
    assert!(
        body["window_since"].is_null(),
        "window_since is null for lifetime, got {:?}",
        body["window_since"]
    );
    assert!(body["orchestration"]["previous_normalized_tokens"].is_null());
    assert_eq!(body["orchestration"]["schema_version"].as_u64(), Some(2));
    assert_eq!(
        body["orchestration"]["normalized_tokens"]["normalized_token_total"].as_u64(),
        Some(0)
    );
    assert_eq!(body["orchestration"]["scope"], "managed_execution");
    assert!(
        chrono::DateTime::parse_from_rfc3339(
            body["orchestration"]["until"]
                .as_str()
                .expect("until timestamp"),
        )
        .expect("parse until")
            <= chrono::DateTime::parse_from_rfc3339(
                body["orchestration"]["as_of"]
                    .as_str()
                    .expect("as_of timestamp"),
            )
            .expect("parse as_of")
    );
    assert!(body["orchestration"]["buckets"].is_array());
}

#[tokio::test]
async fn scoreboard_query_window_1h_populates_window_and_since() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=1h")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["schema_version"].as_u64(), Some(8));
    assert_eq!(body["window"].as_str(), Some("1h"));
    assert!(body["orchestration"]["previous_normalized_tokens"].is_object());
    let since = body["window_since"]
        .as_str()
        .expect("window_since is RFC3339 string for non-all window");
    // Surface check: parses as a RFC3339 timestamp.
    let _ =
        chrono::DateTime::parse_from_rfc3339(since).expect("window_since must be valid RFC3339");
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(
            body["orchestration"]["since"]
                .as_str()
                .expect("orchestration since"),
        )
        .expect("parse orchestration since"),
        chrono::DateTime::parse_from_rfc3339(since).expect("parse scoreboard since"),
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(
            body["orchestration"]["until"]
                .as_str()
                .expect("until timestamp"),
        )
        .expect("parse until")
            <= chrono::DateTime::parse_from_rfc3339(
                body["orchestration"]["as_of"]
                    .as_str()
                    .expect("as_of timestamp"),
            )
            .expect("parse as_of")
    );
}

#[tokio::test]
async fn scoreboard_query_window_bogus_returns_400_with_error_body() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=bogus")).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    let err = body["error"]
        .as_str()
        .expect("400 body has an 'error' string field");
    assert!(
        err.contains("bogus"),
        "error message names the bad input, got {err}"
    );
}

#[tokio::test]
async fn scoreboard_query_window_7d_is_not_a_24h_payload() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=7d")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["window"].as_str(), Some("7d"));
    assert_ne!(
        body["window"].as_str(),
        Some("24h"),
        "a 7d request must not report a 24h window"
    );
    let since = chrono::DateTime::parse_from_rfc3339(
        body["window_since"].as_str().expect("window_since for 7d"),
    )
    .expect("parse window_since");
    let orch_since = chrono::DateTime::parse_from_rfc3339(
        body["orchestration"]["since"]
            .as_str()
            .expect("orchestration since"),
    )
    .expect("parse orchestration since");
    let until = chrono::DateTime::parse_from_rfc3339(
        body["orchestration"]["until"]
            .as_str()
            .expect("orchestration until"),
    )
    .expect("parse until");
    assert_eq!(
        since, orch_since,
        "scoreboard and managed-execution cutoffs must match"
    );
    let span = until.signed_duration_since(orch_since);
    assert!(
        span >= chrono::Duration::days(7) - chrono::Duration::seconds(2)
            && span <= chrono::Duration::days(7) + chrono::Duration::seconds(2),
        "7d orchestration span must be ~7 days, got {span}"
    );
    assert!(
        until
            <= chrono::DateTime::parse_from_rfc3339(
                body["orchestration"]["as_of"].as_str().expect("as_of"),
            )
            .expect("parse as_of")
    );
}

#[tokio::test]
async fn scoreboard_query_window_all_round_trips_explicitly() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=all")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["window"].as_str(), Some("all"));
    assert!(body["window_since"].is_null());
}

/// ORB-10871: the per-agent row carries the grouped incident count next to the
/// raw `failed_tool_calls` it collapsed, scoped to the requested window, so the
/// leaderboard cannot present one burst as many quality failures.
#[tokio::test]
async fn scoreboard_reports_failure_incidents_beside_raw_failed_tool_calls() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    for index in 0..9 {
        runtime
            .record_audit_event(&orbit_core::AuditEventInsertParams {
                execution_id: format!("exec-{index}"),
                command: "tool".to_string(),
                subcommand: Some("run".to_string()),
                tool_name: Some("orbit.surface.alpha".to_string()),
                target_type: Some("tool".to_string()),
                target_id: Some("orbit.surface.alpha".to_string()),
                role: "claude".to_string(),
                status: orbit_core::AuditEventStatus::Failure,
                exit_code: 1,
                duration_ms: 3,
                working_directory: "/tmp/fixture".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: Some(format!("could not remove /work/dir-{index}/file.txt")),
                host: None,
                pid: std::process::id(),
                session_id: None,
                workspace_id: None,
                caller_machine_id: None,
                caller_host_id: None,
                process_machine_id: None,
                process_host_id: None,
                transport: None,
                effective_capabilities: Default::default(),
                origin_session_id: None,
                mcp_call_id: None,
                lease_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("seed audit event");
    }

    let response = get_scoreboard(runtime, Some("window=24h")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    let agent = &body["agents"]["claude"];
    assert_eq!(
        agent["failed_tool_calls"].as_u64(),
        Some(9),
        "the raw failed-call count is preserved"
    );
    assert_eq!(
        agent["failure_incidents"].as_u64(),
        Some(1),
        "nine identical failures are one incident"
    );
    assert_eq!(
        agent["failure_incident_events"].as_u64(),
        Some(9),
        "the incident states how much raw evidence it collapsed"
    );
    assert_eq!(agent["unexpected_failure_incidents"].as_u64(), Some(1));
}
