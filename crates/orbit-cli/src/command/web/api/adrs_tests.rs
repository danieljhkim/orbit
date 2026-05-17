use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::OrbitRuntime;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::super::test_support::body_json;

const ADR_BODY: &str = "## Context\nFixture context.\n\n## Decision\nFixture decision.\n\n## Consequences\n- Dashboard behavior is observable.\n- Cost: Test fixtures carry enough ADR shape to pass validation.\n";

fn seed_adr(runtime: &OrbitRuntime, title: &str, related_tasks: Vec<&str>) -> Value {
    runtime
        .execute_tool_command(
            "orbit.adr.add",
            json!({
                "title": title,
                "body": ADR_BODY,
                "owner": "gpt-5.5",
                "related_features": ["dashboard"],
                "related_tasks": related_tasks,
            }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("seed ADR")
}

fn accept_adr(runtime: &OrbitRuntime, id: &str) -> Value {
    runtime
        .execute_tool_command(
            "orbit.adr.update",
            json!({
                "id": id,
                "status": "accepted",
            }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("accept ADR")
}

fn adr_id(adr: &Value) -> &str {
    adr["id"].as_str().expect("ADR id")
}

async fn request_accept(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/adrs/{id}/accept"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }

    router()
        .with_state(Arc::new(runtime))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_supersede(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/adrs/{id}/supersede"));
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
        .with_state(Arc::new(runtime))
        .oneshot(request)
        .await
        .expect("response")
}

#[tokio::test]
async fn post_adr_routes_require_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let proposed = seed_adr(&runtime, "Proposed dashboard ADR", vec!["ORB-00063"]);

    let response = request_accept(runtime.clone(), adr_id(&proposed), None).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&proposed) }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("show proposed");
    assert_eq!(stored["status"], "proposed");

    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    let new = seed_adr(&runtime, "New dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));
    accept_adr(&runtime, adr_id(&new));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        None,
        Some(json!({ "by": adr_id(&new) })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&old) }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("show old");
    assert_eq!(stored["status"], "accepted");
}

#[tokio::test]
async fn accept_returns_bad_request_when_tool_rejects_missing_related_tasks() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let adr = seed_adr(&runtime, "Needs task linkage", vec![]);

    let response = request_accept(runtime, adr_id(&adr), Some("http://localhost:7878")).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_json(response).await;
    let error = payload["error"].as_str().expect("error");
    assert!(
        error.contains(&format!(
            "Invalid ADR status transition: {}: proposed -> accepted requires non-empty related_tasks",
            adr_id(&adr)
        )),
        "{error}"
    );
}

#[tokio::test]
async fn supersede_returns_not_found_for_unknown_source_id() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let replacement = seed_adr(&runtime, "Replacement dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&replacement));

    let response = request_supersede(
        runtime,
        "ADR-9999",
        Some("http://localhost:7878"),
        Some(json!({ "by": adr_id(&replacement) })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn supersede_rejects_malformed_by() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        Some("http://127.0.0.1:7878"),
        Some(json!({ "by": "bad" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let stored = runtime
        .execute_tool_command(
            "orbit.adr.show",
            json!({ "id": adr_id(&old) }),
            None,
            Some("gpt-5.5".to_string()),
        )
        .expect("show old");
    assert_eq!(stored["status"], "accepted");
}

#[tokio::test]
async fn supersede_moves_source_to_superseded_and_populates_edge() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_adr(&runtime, "Old dashboard ADR", vec!["ORB-00063"]);
    let new = seed_adr(&runtime, "New dashboard ADR", vec!["ORB-00063"]);
    accept_adr(&runtime, adr_id(&old));
    accept_adr(&runtime, adr_id(&new));

    let response = request_supersede(
        runtime.clone(),
        adr_id(&old),
        Some("http://localhost:7878"),
        Some(json!({ "by": adr_id(&new), "reason": "replacement" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["old"]["id"], adr_id(&old));
    assert_eq!(payload["old"]["status"], "superseded");
    assert_eq!(payload["old"]["superseded_by"], adr_id(&new));
    assert_eq!(payload["new"]["id"], adr_id(&new));
    assert_eq!(payload["new"]["supersedes"][0], adr_id(&old));

    let superseded_dir = runtime
        .data_root()
        .join("adrs")
        .join("superseded")
        .join(adr_id(&old));
    assert!(superseded_dir.is_dir(), "{}", superseded_dir.display());
    let accepted_dir = runtime
        .data_root()
        .join("adrs")
        .join("accepted")
        .join(adr_id(&old));
    assert!(!accepted_dir.exists(), "{}", accepted_dir.display());
}
