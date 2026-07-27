// ORB-00337: window-aware scoreboard endpoint contract.
//
// Asserts the HTTP surface for `?window=` honors the scoreboard windowing
// behavior added in orbit-store / orbit-core:
// - missing param defaults to lifetime (`window: "all"`)
// - `?window=1h` round-trips into the serialized payload + populates
//   `window_since`
// - unknown values produce HTTP 400 (not a 500)
// - schema_version is the post-bump v6 value

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
async fn scoreboard_default_returns_lifetime_window_and_v6_schema() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["schema_version"].as_u64(), Some(6));
    assert_eq!(body["window"].as_str(), Some("all"));
    assert!(
        body["window_since"].is_null(),
        "window_since is null for lifetime, got {:?}",
        body["window_since"]
    );
}

#[tokio::test]
async fn scoreboard_query_window_1h_populates_window_and_since() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=1h")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["schema_version"].as_u64(), Some(6));
    assert_eq!(body["window"].as_str(), Some("1h"));
    let since = body["window_since"]
        .as_str()
        .expect("window_since is RFC3339 string for non-all window");
    // Surface check: parses as a RFC3339 timestamp.
    let _ =
        chrono::DateTime::parse_from_rfc3339(since).expect("window_since must be valid RFC3339");
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
async fn scoreboard_query_window_all_round_trips_explicitly() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = get_scoreboard(runtime, Some("window=all")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["window"].as_str(), Some("all"));
    assert!(body["window_since"].is_null());
}
