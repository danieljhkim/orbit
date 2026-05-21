//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use orbit_common::types::TaskArtifact;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitRuntime, TaskComplexity, TaskStatus};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::super::tasks::*;
use super::test_support::body_json;

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
                    created_by: None,
                }],
                ..Default::default()
            },
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
        )
        .expect("upsert artifact")
}

fn seed_lock_task(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    context_files: Vec<&str>,
    job_run_id: Option<&str>,
) -> orbit_core::Task {
    for selector in &context_files {
        if let Some(path) = selector.strip_prefix("file:") {
            let path = runtime
                .data_root()
                .parent()
                .expect("runtime data root has repo parent")
                .join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create context parent");
            }
            std::fs::write(path, "").expect("write context file");
        }
    }
    let task = runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture for {title}."),
            status: Some(status),
            context_files: context_files.into_iter().map(str::to_string).collect(),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("create lock task");
    if let Some(job_run_id) = job_run_id {
        runtime
            .update_task_with_identity(
                &task.id,
                TaskUpdateParams {
                    job_run_id: Some(Some(job_run_id.to_string())),
                    ..Default::default()
                },
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("set job run")
    } else {
        task
    }
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
async fn task_locks_endpoint_matches_cli_json_contract() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let review = seed_lock_task(
        &runtime,
        "Review task",
        TaskStatus::Review,
        vec!["file:src/b.rs", "file:src/shared.rs"],
        Some("jrun-review"),
    );
    let in_progress = seed_lock_task(
        &runtime,
        "In progress task",
        TaskStatus::InProgress,
        vec!["file:src/a.rs", "file:src/shared.rs"],
        None,
    );
    seed_lock_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        vec!["file:src/ignored.rs"],
        None,
    );
    seed_lock_task(
        &runtime,
        "Done task",
        TaskStatus::Done,
        vec!["file:src/done.rs"],
        None,
    );
    let expected = crate::projections::task_locks_json(&runtime).expect("cli task locks json");

    let response = request(runtime, "/tasks/locks").await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body, expected);
    assert_eq!(
        body["locked_files"],
        json!(["file:src/a.rs", "file:src/b.rs", "file:src/shared.rs"])
    );
    assert_eq!(body["total_locked"], json!(3));
    assert_eq!(body["total_tasks"], json!(2));
    let by_task = body["by_task"].as_array().expect("by_task array");
    assert_eq!(by_task[0]["id"], json!(in_progress.id));
    assert_eq!(by_task[1]["id"], json!(review.id));
    assert!(
        !by_task
            .iter()
            .any(|task| task["status"] == json!("backlog"))
    );
    assert!(!by_task.iter().any(|task| task["status"] == json!("done")));
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

/// Exercises PATCH /api/tasks/:id with the dashboard's emitted spelling {"status":"in-progress"}
/// against a backlog task. Before the serde alias fix this produced 422 on JSON extraction;
/// now it succeeds and the response continues to surface status as the display form "in-progress".
#[tokio::test]
async fn patch_api_accepts_in_progress_hyphen_from_dashboard_and_returns_in_progress() {
    use axum::Router;
    use axum::http::{Method, Request, header};

    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let created = runtime
        .add_task(TaskAddParams {
            title: "Dashboard status update test".to_string(),
            description: "backlog task to be moved via PATCH with hyphen spelling".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("seed backlog task");
    let task_id = created.id;

    // Wrap to exercise the literal /api/tasks path per acceptance criteria
    let app = Router::new()
        .nest("/api", router())
        .with_state(Arc::new(runtime));

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/api/tasks/{}", task_id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:7878")
                .body(Body::from(r#"{"status":"in-progress"}"#))
                .expect("build patch request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "PATCH with in-progress must succeed (not 422)"
    );

    let body = body_json(response).await;
    assert_eq!(body["id"], serde_json::json!(task_id));
    assert_eq!(
        body["status"],
        serde_json::json!("in-progress"),
        "response must continue to expose dashboard display spelling"
    );
}

/// Contract test: /tasks projection (and /tasks/:id) must include `complexity` string
/// when TaskComplexity is set on the task (low/medium/hard). Null complexity omits the key
/// or yields null (per current projection); this test asserts presence for a hard task.
#[tokio::test]
async fn list_tasks_includes_complexity_when_set() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let with_complexity = runtime
        .add_task(TaskAddParams {
            title: "Hard task for complexity display".to_string(),
            description: "Task with explicit complexity for dashboard test.".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            complexity: Some(TaskComplexity::Hard),
            ..Default::default()
        })
        .expect("seed task with complexity");
    // Also seed one without to ensure list works
    let _without = runtime
        .add_task(TaskAddParams {
            title: "Plain task no complexity".to_string(),
            description: "no complexity set".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("seed plain task");

    let response = request(runtime, "/tasks").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let arr = body.as_array().expect("tasks list is array");
    let found = arr
        .iter()
        .find(|t| t["id"] == serde_json::json!(with_complexity.id))
        .expect("task present in /tasks");
    assert_eq!(
        found.get("complexity"),
        Some(&serde_json::json!("hard")),
        "complexity must be projected as string for dashboard"
    );
}
