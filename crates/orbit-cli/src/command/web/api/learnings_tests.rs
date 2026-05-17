use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::{
    EvidenceKind, Learning, LearningCreateParams, LearningEvidence, LearningScope, LearningStatus,
    OrbitRuntime,
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::super::test_support::body_json;

fn seed_learning(runtime: &OrbitRuntime, summary: &str) -> Learning {
    runtime
        .create_learning(LearningCreateParams {
            summary: summary.to_string(),
            scope: LearningScope {
                paths: vec!["crates/orbit-cli/**".to_string()],
                tags: vec!["dashboard".to_string()],
                ..Default::default()
            },
            body: format!("Body for {summary}."),
            evidence: vec![LearningEvidence {
                kind: EvidenceKind::Task,
                reference: "ORB-00061".to_string(),
            }],
            created_by: Some("gpt-5.5".to_string()),
            priority: Some(3),
        })
        .expect("seed learning")
}

async fn request_supersede(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/learnings/{id}/supersede"));
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
async fn supersede_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_learning(&runtime, "Old dashboard learning");
    let new = seed_learning(&runtime, "New dashboard learning");

    let response = request_supersede(
        runtime.clone(),
        &old.id,
        None,
        Some(json!({ "by": new.id })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime.get_learning(&old.id).expect("read old");
    assert_eq!(stored.status, LearningStatus::Active);
    assert_eq!(stored.superseded_by, None);
}

#[tokio::test]
async fn supersede_rejects_missing_or_malformed_by() {
    let cases = [
        (json!({}), "missing by"),
        (json!({ "by": "" }), "empty by"),
        (json!({ "by": "bad" }), "malformed by"),
    ];

    for (body, label) in cases {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let old = seed_learning(&runtime, "Old dashboard learning");

        let response = request_supersede(
            runtime.clone(),
            &old.id,
            Some("http://localhost:7878"),
            Some(body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        let stored = runtime.get_learning(&old.id).expect("read old");
        assert_eq!(stored.status, LearningStatus::Active, "{label}");
        assert_eq!(stored.superseded_by, None, "{label}");
    }
}

#[tokio::test]
async fn supersede_returns_not_found_when_target_id_is_missing() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let replacement = seed_learning(&runtime, "Replacement dashboard learning");

    let response = request_supersede(
        runtime,
        "L20260516-9999",
        Some("http://localhost:7878"),
        Some(json!({ "by": replacement.id })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn supersede_updates_target_record() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_learning(&runtime, "Old dashboard learning");
    let new = seed_learning(&runtime, "New dashboard learning");

    let response = request_supersede(
        runtime.clone(),
        &old.id,
        Some("http://127.0.0.1:7878"),
        Some(json!({ "by": new.id, "reason": "duplicate" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["old"]["id"], old.id);
    assert_eq!(payload["old"]["status"], "superseded");
    assert_eq!(payload["old"]["superseded_by"], new.id);

    let stored = runtime.get_learning(&old.id).expect("read superseded");
    assert_eq!(stored.status, LearningStatus::Superseded);
    assert_eq!(stored.superseded_by.as_deref(), Some(new.id.as_str()));
}

#[tokio::test]
async fn list_learnings_returns_stats_and_rows() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_learning(&runtime, "Old dashboard learning");
    let new = seed_learning(&runtime, "New dashboard learning");
    runtime
        .supersede_learning(&old.id, &new.id)
        .expect("supersede fixture");

    let response = router()
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/learnings")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["stats"]["total"], 2);
    assert_eq!(payload["stats"]["superseded"], 1);
    assert!(payload["stats"]["last_indexed"].as_str().is_some());
    assert_eq!(payload["items"].as_array().expect("items").len(), 2);
}
