use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use orbit_common::types::TaskArtifact;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitRuntime, TaskStatus};
use serde_json::Value;
use tower::ServiceExt;

use super::test_support::body_json;
use super::*;

fn seed_task_with_artifact(runtime: &OrbitRuntime) -> orbit_core::Task {
    let task = runtime
        .add_task(TaskAddParams {
            title: "Artifact task".to_string(),
            description: "Fixture task with an artifact.".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("create task");
    runtime
        .update_task_with_identity(
            &task.id,
            TaskUpdateParams {
                upsert_artifacts: vec![TaskArtifact {
                    path: "subdir/file.json".to_string(),
                    media_type: "application/json".to_string(),
                    content: br#"{"ok":true}"#.to_vec(),
                }],
                ..Default::default()
            },
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("upsert artifact")
}

async fn request(runtime: OrbitRuntime, uri: &str) -> axum::response::Response {
    router()
        .with_state(Arc::new(runtime))
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

#[tokio::test]
async fn get_task_projects_artifact_manifest_without_content() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);

    let response = request(runtime, &format!("/tasks/{}", task.id)).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    assert_eq!(artifacts.len(), 1);
    let artifact = artifacts.first().expect("artifact");
    let object = artifact.as_object().expect("artifact object");
    let keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "created_at",
            "created_by",
            "media_type",
            "path",
            "sha256",
            "size_bytes"
        ]
    );
    assert_eq!(
        artifact["path"],
        Value::String("subdir/file.json".to_string())
    );
    assert_eq!(
        artifact["media_type"],
        Value::String("application/json".to_string())
    );
    assert_eq!(
        artifact["size_bytes"],
        Value::Number(serde_json::Number::from(11))
    );
    assert!(artifact.get("content").is_none());
}

#[tokio::test]
async fn get_task_artifact_serves_subdirectory_bytes_and_media_type() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/subdir/file.json", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], br#"{"ok":true}"#);
}

#[tokio::test]
async fn get_task_artifact_returns_not_found_for_missing_artifact() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/missing.json", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn get_task_artifact_rejects_traversal_path() {
    tokio::runtime::Runtime::new()
        .expect("build tokio runtime")
        .block_on(async {
            let runtime = OrbitRuntime::in_memory().expect("build runtime");
            let task = seed_task_with_artifact(&runtime);

            let response = request(
                runtime,
                &format!("/tasks/{}/artifacts/subdir/%2e%2e/%2e%2e/escape", task.id),
            )
            .await;

            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        });
}
