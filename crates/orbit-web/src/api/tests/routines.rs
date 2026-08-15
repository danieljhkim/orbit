//! Tests for the routine-health JSON API [ORB-10138].

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use orbit_core::{RoutineFireRecord, RoutineFireState};
use orbit_registry::{NewHostIdentity, ensure_host_identity};
use tower::ServiceExt;

use super::super::router;
use super::super::routines::{duration_ms, fire_json, fire_ok};
use super::test_support::body_json;
use crate::state::DashboardState;

fn fire(state: RoutineFireState, created_at: &str, updated_at: &str) -> RoutineFireRecord {
    RoutineFireRecord {
        routine_name: "ship-sweep".to_string(),
        slot: "2026-07-11T22:00:00+00:00".to_string(),
        attempt: 1,
        state,
        run_id: Some("run-1".to_string()),
        source_workspace: "polaris".to_string(),
        detail: None,
        created_at: created_at.to_string(),
        updated_at: updated_at.to_string(),
    }
}

#[test]
fn fire_ok_classifies_terminal_and_in_flight_states() {
    assert_eq!(fire_ok(RoutineFireState::Succeeded), Some(true));
    assert_eq!(fire_ok(RoutineFireState::Failed), Some(false));
    assert_eq!(fire_ok(RoutineFireState::TimedOut), Some(false));
    assert_eq!(fire_ok(RoutineFireState::Error), Some(false));
    assert_eq!(fire_ok(RoutineFireState::Intent), None);
    assert_eq!(fire_ok(RoutineFireState::Dispatched), None);
}

#[test]
fn duration_ms_spans_intent_to_terminal() {
    let f = fire(
        RoutineFireState::Succeeded,
        "2026-07-11T22:00:00+00:00",
        "2026-07-11T22:00:12.500+00:00",
    );
    assert_eq!(duration_ms(&f), Some(12_500));
}

#[test]
fn duration_ms_is_none_while_in_flight() {
    let f = fire(
        RoutineFireState::Dispatched,
        "2026-07-11T22:00:00+00:00",
        "2026-07-11T22:00:05+00:00",
    );
    assert_eq!(duration_ms(&f), None);
}

#[test]
fn fire_json_surfaces_outcome_and_finish() {
    let f = fire(
        RoutineFireState::Succeeded,
        "2026-07-11T22:00:00+00:00",
        "2026-07-11T22:00:10+00:00",
    );
    let json = fire_json(&f);
    assert_eq!(json["state"], "succeeded");
    assert_eq!(json["ok"], true);
    assert_eq!(json["duration_ms"], 10_000);
    assert_eq!(json["finished_at"], "2026-07-11T22:00:10+00:00");
    assert_eq!(json["run_id"], "run-1");
}

#[test]
fn fire_json_omits_finish_while_in_flight() {
    let f = fire(
        RoutineFireState::Dispatched,
        "2026-07-11T22:00:00+00:00",
        "2026-07-11T22:00:00+00:00",
    );
    let json = fire_json(&f);
    assert!(json["ok"].is_null());
    assert!(json["finished_at"].is_null());
    assert!(json["duration_ms"].is_null());
}

/// End-to-end: the endpoint resolves host-level routine state from the global
/// root and returns a well-formed envelope even when no routines are
/// configured (empty registry). Exercises the wiring, not fixture data.
#[tokio::test]
async fn routines_endpoint_returns_envelope_for_empty_host() {
    let temp = tempfile::tempdir().expect("temp global root");
    ensure_host_identity(temp.path(), || {
        Ok(NewHostIdentity {
            host_id: "dashboard-test".to_string(),
            task_prefix: "DA".to_string(),
        })
    })
    .expect("seed host identity");
    let state = DashboardState::global(temp.path().to_path_buf(), Vec::new(), None);

    let response = router()
        .with_state(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/routines")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert!(json["generated_at"].is_string());
    assert!(json["host_id"].is_string());
    assert_eq!(json["routines"], serde_json::json!([]));
    assert_eq!(json["load_errors"], serde_json::json!([]));
}
