//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::OrbitRuntime;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

fn seed_friction(runtime: &OrbitRuntime, body: &str, tags: &[&str]) -> Value {
    runtime
        .run_tool(
            "orbit.friction.add",
            json!({
                "body": body,
                "tags": tags,
                "model": "gpt-5.5",
            }),
        )
        .expect("seed friction")
}

async fn request(
    runtime: OrbitRuntime,
    method: Method,
    uri: String,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };

    router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(request)
        .await
        .expect("response")
}

#[tokio::test]
async fn patch_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let friction = seed_friction(&runtime, "# Build snag\nDetails.", &["tooling"]);
    let id = friction["id"].as_str().expect("friction id");

    for (origin, label) in [(None, "missing"), (Some("http://example.com"), "foreign")] {
        let response = request(
            runtime.clone(),
            Method::PATCH,
            format!("/frictions/{id}"),
            origin,
            Some(json!({ "status": "triaged" })),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
    }

    let response = request(runtime, Method::GET, format!("/frictions/{id}"), None, None).await;
    let payload = body_json(response).await;
    assert_eq!(payload["status"], "open");
}

#[tokio::test]
async fn resolve_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let friction = seed_friction(&runtime, "# Policy snag\nDetails.", &["policy"]);
    let id = friction["id"].as_str().expect("friction id");

    for (origin, label) in [(None, "missing"), (Some("http://example.com"), "foreign")] {
        let response = request(
            runtime.clone(),
            Method::POST,
            format!("/frictions/{id}/resolve"),
            origin,
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
    }

    let response = request(runtime, Method::GET, format!("/frictions/{id}"), None, None).await;
    let payload = body_json(response).await;
    assert_eq!(payload["status"], "open");
}

#[tokio::test]
async fn path_traversal_id_is_rejected() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = request(
        runtime,
        Method::GET,
        "/frictions/%2e%2e".to_string(),
        None,
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn patch_round_trip_updates_status_and_tags() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let friction = seed_friction(&runtime, "# Slow docs\nDetails.", &["tooling"]);
    let id = friction["id"].as_str().expect("friction id");

    let response = request(
        runtime.clone(),
        Method::PATCH,
        format!("/frictions/{id}"),
        Some("http://localhost:7878"),
        Some(json!({
            "status": "triaged",
            "tags": ["docs"],
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let response = request(runtime, Method::GET, format!("/frictions/{id}"), None, None).await;
    let payload = body_json(response).await;
    assert_eq!(payload["status"], "triaged");
    assert_eq!(payload["tags"], json!(["docs"]));
}

#[tokio::test]
async fn stats_shape_exposes_triage_counts() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_friction(&runtime, "# Open one\nDetails.", &["tooling"]);
    let triaged = seed_friction(&runtime, "# Triaged one\nDetails.", &["docs"]);
    let resolved = seed_friction(&runtime, "# Resolved one\nDetails.", &["policy"]);
    runtime
        .run_tool(
            "orbit.friction.update",
            json!({
                "id": triaged["id"],
                "status": "triaged",
                "model": "gpt-5.5",
            }),
        )
        .expect("triage friction");
    runtime
        .run_tool(
            "orbit.friction.resolve",
            json!({
                "id": resolved["id"],
                "model": "gpt-5.5",
            }),
        )
        .expect("resolve friction");

    let response = request(
        runtime,
        Method::GET,
        "/frictions/stats".to_string(),
        None,
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    for key in ["open", "triaged", "resolved_this_month"] {
        assert!(
            payload.get(key).and_then(Value::as_u64).is_some(),
            "{key} should be a u64 in {payload}"
        );
    }
}
