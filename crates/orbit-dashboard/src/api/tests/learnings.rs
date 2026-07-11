//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_common::test_fixtures::TEST_CODEX_MODEL;
use orbit_core::{
    EvidenceKind, Learning, LearningCreateParams, LearningEvidence, LearningScope, LearningStatus,
    OrbitRuntime,
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

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
            created_by: Some(TEST_CODEX_MODEL.to_string()),
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
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
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
        "L-9999",
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
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
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

async fn request_update(
    runtime: OrbitRuntime,
    id: &str,
    origin: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/learnings/{id}"));
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
async fn update_requires_localhost_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let learning = seed_learning(&runtime, "Cross-origin learning");

    let response = request_update(
        runtime.clone(),
        &learning.id,
        None,
        Some(json!({ "summary": "Updated" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime.get_learning(&learning.id).expect("read learning");
    assert_eq!(stored.summary, "Cross-origin learning");
}

#[tokio::test]
async fn update_rejects_empty_body() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let learning = seed_learning(&runtime, "Empty patch learning");

    let response = request_update(
        runtime,
        &learning.id,
        Some("http://localhost:7878"),
        Some(json!({})),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_replaces_summary_and_scope_tags() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let learning = seed_learning(&runtime, "Original learning");

    let response = request_update(
        runtime.clone(),
        &learning.id,
        Some("http://localhost:7878"),
        Some(json!({
            "summary": "Curated learning",
            "scope": { "paths": ["crates/orbit-dashboard/**"], "tags": ["api", "http"] },
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_json(response).await;
    assert_eq!(payload["summary"], "Curated learning");

    let stored = runtime.get_learning(&learning.id).expect("read learning");
    assert_eq!(stored.summary, "Curated learning");
    assert_eq!(
        stored.scope.tags,
        vec!["api".to_string(), "http".to_string()]
    );
    assert_eq!(stored.status, LearningStatus::Active);
}

#[tokio::test]
async fn update_rejects_superseded_record() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let old = seed_learning(&runtime, "Old learning");
    let new = seed_learning(&runtime, "New learning");
    runtime
        .supersede_learning(&old.id, &new.id)
        .expect("supersede fixture");

    // A superseded learning must never be mutated in place — the lifecycle is
    // supersede-don't-delete, and updates to it are rejected.
    let response = request_update(
        runtime.clone(),
        &old.id,
        Some("http://localhost:7878"),
        Some(json!({ "summary": "Should not apply" })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let stored = runtime.get_learning(&old.id).expect("read superseded");
    assert_eq!(stored.status, LearningStatus::Superseded);
    assert_eq!(stored.summary, "Old learning");
}
