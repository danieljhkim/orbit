//! Test-only allowlist: the original tests under orbit-cli passed the same lints via
//! the crate-level test harness configuration; duplicated here for the extracted crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use orbit_common::test_fixtures::TEST_CODEX_MODEL;
use orbit_common::types::TaskArtifact;
use orbit_common::types::task_artifacts::{TaskRelation, TaskRelationType};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitRuntime, TaskComplexity, TaskStatus};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::super::router;
use super::test_support::body_json;

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::ORIGIN, "http://localhost:7878")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn patch_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(uri)
        .header(header::ORIGIN, "http://localhost:7878")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn seed_backlog_task(runtime: &OrbitRuntime, title: &str) -> orbit_core::Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}."),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("seed backlog task")
}

fn seed_task_with_artifact(runtime: &OrbitRuntime) -> orbit_core::Task {
    seed_task_with_artifact_payload(
        runtime,
        "subdir/file.json",
        "application/json",
        br#"{"ok":true}"#.to_vec(),
    )
}

fn seed_task_with_artifact_payload(
    runtime: &OrbitRuntime,
    path: &str,
    media_type: &str,
    content: Vec<u8>,
) -> orbit_core::Task {
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
                    path: path.to_string(),
                    media_type: media_type.to_string(),
                    content,
                    created_by: None,
                }],
                ..Default::default()
            },
            Some("codex".to_string()),
            Some(TEST_CODEX_MODEL.to_string()),
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
                Some(TEST_CODEX_MODEL.to_string()),
            )
            .expect("set job run")
    } else {
        task
    }
}

async fn request(runtime: OrbitRuntime, uri: &str) -> axum::response::Response {
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
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .is_none()
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], br#"{"ok":true}"#);
}

#[tokio::test]
async fn get_task_artifact_normalizes_text_plain_and_keeps_it_inline() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact_payload(
        &runtime,
        "notes/output.txt",
        "Text/Plain; Charset=UTF-8",
        b"plain artifact".to_vec(),
    );

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/notes/output.txt", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/plain"))
    );
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .is_none()
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], b"plain artifact");
}

#[tokio::test]
async fn get_task_artifact_downloads_html_instead_of_serving_inline() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact_payload(
        &runtime,
        "reports/payload.html",
        "text/html; charset=utf-8",
        br#"<script>fetch("/api/tasks")</script>"#.to_vec(),
    );

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/reports/payload.html", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/octet-stream"))
    );
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert_eq!(
        response.headers().get(header::CONTENT_DISPOSITION),
        Some(&HeaderValue::from_static("attachment"))
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], br#"<script>fetch("/api/tasks")</script>"#);
}

#[tokio::test]
async fn get_task_artifact_downloads_script_media_types() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact_payload(
        &runtime,
        "reports/payload.js",
        "application/javascript",
        b"fetch('/api/tasks')".to_vec(),
    );

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/reports/payload.js", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/octet-stream"))
    );
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert_eq!(
        response.headers().get(header::CONTENT_DISPOSITION),
        Some(&HeaderValue::from_static("attachment"))
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], b"fetch('/api/tasks')");
}

#[tokio::test]
async fn get_task_artifact_downloads_unknown_media_types() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact_payload(
        &runtime,
        "reports/payload.custom",
        "application/x-orbit-preview",
        b"custom artifact".to_vec(),
    );

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/reports/payload.custom", task.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/octet-stream"))
    );
    assert_eq!(
        response.headers().get("x-content-type-options"),
        Some(&HeaderValue::from_static("nosniff"))
    );
    assert_eq!(
        response.headers().get(header::CONTENT_DISPOSITION),
        Some(&HeaderValue::from_static("attachment"))
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(&bytes[..], b"custom artifact");
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

const SECRET_CONTENT: &str = "TOP-SECRET-OUTSIDE-ARTIFACT-ROOT";

/// Plants a secret file *outside* the artifact root (next to the runtime's
/// data root) and returns its absolute path, so adversarial requests have a
/// concrete escape target whose bytes must never appear in a response.
fn plant_secret_outside_artifact_root(runtime: &OrbitRuntime) -> std::path::PathBuf {
    let secret_path = runtime.data_root().join("secret-fixture.txt");
    std::fs::write(&secret_path, SECRET_CONTENT).expect("write secret fixture");
    secret_path
}

async fn assert_artifact_request_denied(response: axum::response::Response, label: &str) {
    let status = response.status();
    assert!(
        status.is_client_error(),
        "{label}: expected 4xx, got {status}"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        !body.contains(SECRET_CONTENT),
        "{label}: response leaked out-of-root file content"
    );
}

/// ORB-10008: adversarial path shapes against the artifact validator. Every
/// case must be a clean 4xx and must never serve bytes from outside the task's
/// artifact root.
#[tokio::test]
async fn get_task_artifact_rejects_adversarial_paths() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);
    plant_secret_outside_artifact_root(&runtime);

    let cases: &[(&str, String)] = &[
        (
            "raw dot-dot traversal",
            format!(
                "/tasks/{}/artifacts/../../../../secret-fixture.txt",
                task.id
            ),
        ),
        (
            "encoded dot-dot traversal",
            format!(
                "/tasks/{}/artifacts/subdir/%2e%2e%2f%2e%2e%2fsecret-fixture.txt",
                task.id
            ),
        ),
        (
            "encoded absolute path",
            format!("/tasks/{}/artifacts/%2Fetc%2Fpasswd", task.id),
        ),
        (
            "backslash separators",
            format!(
                "/tasks/{}/artifacts/subdir%5C..%5C..%5Csecret-fixture.txt",
                task.id
            ),
        ),
        (
            "leading current-dir component",
            format!("/tasks/{}/artifacts/.%2Fsubdir%2Ffile.json", task.id),
        ),
    ];
    for (label, uri) in cases {
        let response = request(runtime.clone(), uri).await;
        assert_artifact_request_denied(response, label).await;
    }

    // A double-slash absolute spelling may 400 (validator) or 404 (router);
    // either way it must not leak file contents.
    let response = request(
        runtime.clone(),
        &format!("/tasks/{}/artifacts//etc/passwd", task.id),
    )
    .await;
    let status = response.status();
    assert!(
        status.is_client_error(),
        "double-slash absolute: expected 4xx, got {status}"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert!(!String::from_utf8_lossy(&bytes).contains("root:"));
}

/// Replace the on-disk blob backing `subdir/file.json` with a symlink to
/// `target`, returning the blob path that was swapped.
#[cfg(unix)]
fn swap_artifact_blob_for_symlink(
    runtime: &OrbitRuntime,
    target: &std::path::Path,
) -> std::path::PathBuf {
    let blob_path = find_artifact_blob(&runtime.data_root(), "file.json")
        .expect("artifact blob exists on disk");
    assert!(
        blob_path.components().any(|c| c.as_os_str() == "artifacts"),
        "blob must live under the artifact directory: {}",
        blob_path.display()
    );
    std::fs::remove_file(&blob_path).expect("remove artifact blob");
    std::os::unix::fs::symlink(target, &blob_path).expect("plant escaping symlink");
    blob_path
}

/// ORB-10008: a manifest-listed blob replaced on disk by a symlink pointing
/// outside the artifact root must be refused, not followed.
///
/// The v2 bundle read sha256/size-verifies every manifest entry before any
/// artifact is served, so the path-containment check (canonicalize +
/// `starts_with`) is only reachable when the escape target is byte-identical
/// to the recorded artifact. This test plants exactly that worst case and
/// asserts the containment validator still rejects the escaping link.
#[cfg(unix)]
#[tokio::test]
async fn get_task_artifact_refuses_symlink_escaping_artifact_root() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);
    // Byte-identical to the seeded artifact so integrity checks pass and the
    // containment check itself is exercised.
    let outside_twin = runtime.data_root().join("outside-twin.json");
    std::fs::write(&outside_twin, br#"{"ok":true}"#).expect("write outside twin");
    swap_artifact_blob_for_symlink(&runtime, &outside_twin);

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/subdir/file.json", task.id),
    )
    .await;

    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unexpected response: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("outside the task artifact directory")),
        "unexpected error body: {body}"
    );
}

/// ORB-10008: when the escaping symlink points at *different* content, the
/// bundle-read integrity verification (sha256/size against the manifest)
/// fails closed before the containment check is reached. The response is an
/// error either way and must never carry the out-of-root bytes.
#[cfg(unix)]
#[tokio::test]
async fn get_task_artifact_fails_closed_on_symlink_with_foreign_content() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_task_with_artifact(&runtime);
    let secret_path = plant_secret_outside_artifact_root(&runtime);
    swap_artifact_blob_for_symlink(&runtime, &secret_path);

    let response = request(
        runtime,
        &format!("/tasks/{}/artifacts/subdir/file.json", task.id),
    )
    .await;

    let status = response.status();
    assert!(
        !status.is_success(),
        "tampered artifact must not be served: got {status}"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert!(
        !String::from_utf8_lossy(&bytes).contains(SECRET_CONTENT),
        "response leaked out-of-root file content"
    );
}

/// Depth-first search for a file named `name` under `root`, skipping nothing.
/// Test-only helper: the artifact bundle layout is an implementation detail of
/// orbit-store, so the test discovers the blob instead of hardcoding the path.
fn find_artifact_blob(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_artifact_blob(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|f| f == name)
            && path.components().any(|c| c.as_os_str() == "artifacts")
        {
            return Some(path);
        }
    }
    None
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
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)));

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

#[tokio::test]
async fn patch_api_persists_pr_status_with_status_and_execution_summary() {
    use axum::http::{Method, Request, header};

    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Dashboard PR status update".to_string(),
            description: "An HTTP update must persist every supplied field.".to_string(),
            status: Some(TaskStatus::InProgress),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("seed in-progress task");

    let response = router()
        .with_state(crate::state::DashboardState::single(Arc::new(
            runtime.clone(),
        )))
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/tasks/{}", task.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:7878")
                .body(Body::from(
                    json!({
                        "pr_status": "approved",
                        "execution_summary": "Implemented and verified.",
                        "status": "review",
                    })
                    .to_string(),
                ))
                .expect("build patch request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["pr_status"], json!("approved"));
    assert_eq!(
        body["execution_summary"],
        json!("Implemented and verified.")
    );
    assert_eq!(body["status"], json!("review"));

    let persisted = runtime.get_task(&task.id).expect("read updated task");
    assert_eq!(persisted.pr_status.as_deref(), Some("approved"));
    assert_eq!(persisted.execution_summary, "Implemented and verified.");
    assert_eq!(persisted.status, TaskStatus::Review);
}

#[tokio::test]
async fn patch_api_persists_complexity_and_omission_preserves_it() {
    use axum::http::{Method, Request, header};

    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = seed_backlog_task(&runtime, "Dashboard complexity update");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(
        runtime.clone(),
    )));

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/tasks/{}", task.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:7878")
                .body(Body::from(json!({ "complexity": "medium" }).to_string()))
                .expect("build complexity patch request"),
        )
        .await
        .expect("complexity patch response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["complexity"], json!("medium"));
    assert_eq!(
        runtime
            .get_task(&task.id)
            .expect("read updated task")
            .complexity,
        Some(TaskComplexity::Medium)
    );

    let response = router()
        .with_state(crate::state::DashboardState::single(Arc::new(
            runtime.clone(),
        )))
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/tasks/{}", task.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:7878")
                .body(Body::from(
                    json!({ "title": "Retitled without complexity" }).to_string(),
                ))
                .expect("build omitted complexity patch request"),
        )
        .await
        .expect("omitted complexity patch response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["complexity"], json!("medium"));
    assert_eq!(
        runtime
            .get_task(&task.id)
            .expect("read preserved complexity")
            .complexity,
        Some(TaskComplexity::Medium)
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

fn seed_task_with_status(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
) -> orbit_core::Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}."),
            status: Some(status),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("seed task")
}

/// ORB-10310: `GET /tasks` is status-neutral — `done` and `archived` tasks
/// (previously excluded by the hard-coded dashboard status subset) are now
/// discoverable, and the list is ordered newest-first.
#[tokio::test]
async fn list_tasks_includes_done_and_archived_statuses_newest_first() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let proposed = seed_task_with_status(&runtime, "Proposed task", TaskStatus::Proposed);
    let done = seed_task_with_status(&runtime, "Done task", TaskStatus::Done);
    let to_archive = seed_task_with_status(&runtime, "Archived task", TaskStatus::Backlog);
    runtime.archive_task(&to_archive.id).expect("archive task");

    let response = request(runtime, "/tasks").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("tasks list is array");

    for expected in [&proposed.id, &done.id, &to_archive.id] {
        assert!(
            rows.iter().any(|row| row["id"].as_str() == Some(expected)),
            "task {expected} must be discoverable regardless of status: {rows:?}"
        );
    }
    let archived_row = rows
        .iter()
        .find(|row| row["id"].as_str() == Some(to_archive.id.as_str()))
        .expect("archived task present");
    assert_eq!(archived_row["status"], json!("archived"));

    let created_ats = rows
        .iter()
        .map(|row| row["created_at"].as_str().expect("created_at").to_string())
        .collect::<Vec<_>>();
    for pair in created_ats.windows(2) {
        assert!(
            pair[0] >= pair[1],
            "tasks must be ordered newest-first: {created_ats:?}"
        );
    }
}

/// ORB-10310: `GET /tasks` bounds the response to the default limit (50),
/// keeping the newest matching tasks.
#[tokio::test]
async fn list_tasks_is_bounded_to_default_limit() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let mut oldest = None;
    for index in 0..55 {
        let task = seed_task_with_status(
            &runtime,
            &format!("Bounded dashboard task {index:02}"),
            TaskStatus::Backlog,
        );
        if index == 0 {
            oldest = Some(task.id);
        }
    }

    let response = request(runtime, "/tasks").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("tasks list is array");
    assert_eq!(rows.len(), 50, "default limit bounds the response to 50");
    let oldest = oldest.expect("seeded at least one task");
    assert!(
        !rows
            .iter()
            .any(|row| row["id"].as_str() == Some(oldest.as_str())),
        "the oldest task must fall outside the newest 50"
    );
}

/// ORB-00042: `workspace` is a selector and lives in the query string, never
/// the body. A stray `workspace` body key (the historical bridge mis-key) must
/// be a loud 400 pointing at `?workspace=`, not silently dropped.
#[tokio::test]
async fn create_task_rejects_stray_workspace_body_key() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let response = router()
        .with_state(crate::state::DashboardState::single(Arc::new(runtime)))
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/tasks")
                .header(header::ORIGIN, "http://localhost:7878")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "title": "mis-keyed",
                        "description": "workspace does not belong in the body",
                        "workspace": "ws_polaris",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    let message = body["error"].as_str().expect("error message");
    assert!(message.contains("workspace"), "names the offending key");
    assert!(message.contains("?workspace="), "points at the query param");
}

#[tokio::test]
async fn create_task_only_accepts_creation_legal_statuses_and_ignores_comment() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(runtime)));

    let rejected = app
        .clone()
        .oneshot(post_json(
            "/tasks",
            json!({
                "title": "illegal status",
                "description": "must not start done",
                "status": "done",
            }),
        ))
        .await
        .expect("response");
    assert!(rejected.status().is_client_error());

    let created = app
        .oneshot(post_json(
            "/tasks",
            json!({
                "title": "legal status",
                "description": "starts in backlog",
                "status": "backlog",
                "comment": "retired create input",
            }),
        ))
        .await
        .expect("response");
    assert_eq!(created.status(), StatusCode::OK);
    let body = body_json(created).await;
    assert_eq!(body["status"], json!("backlog"));
    assert_eq!(body["comments"], json!([]));
}

/// ORB-10253: `POST /api/tasks` accepts a `relations` array of {type, target}
/// objects and persists them on the created task (mirroring the native MCP
/// wire shape). Previously `create_task_action` hardcoded `relations: Vec::new()`.
#[tokio::test]
async fn create_task_persists_relations_from_body() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let target = seed_backlog_task(&runtime, "Relation target");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(runtime)));

    let response = app
        .oneshot(post_json(
            "/tasks",
            json!({
                "title": "task with relations",
                "description": "records a typed relation on create",
                "status": "backlog",
                "relations": [{ "type": "related_to", "target": target.id }],
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body["relations"],
        json!([{ "type": "related_to", "target": target.id }])
    );
}

/// ORB-10253: a present `relations` array on `PATCH /api/tasks/:id` replaces the
/// existing relation set. Previously `update_task_action` hardcoded
/// `relations: None`, so relations could never be edited over HTTP.
#[tokio::test]
async fn update_task_replaces_relation_set() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let first = seed_backlog_task(&runtime, "First relation target");
    let second = seed_backlog_task(&runtime, "Second relation target");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Task with an initial relation".to_string(),
            description: "relation set will be replaced over HTTP".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            relations: vec![TaskRelation {
                relation_type: TaskRelationType::RelatedTo,
                target: first.id.clone(),
            }],
            ..Default::default()
        })
        .expect("seed task with relation");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(runtime)));

    let response = app
        .oneshot(patch_json(
            &format!("/tasks/{}", task.id),
            json!({ "relations": [{ "type": "related_to", "target": second.id }] }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body["relations"],
        json!([{ "type": "related_to", "target": second.id }]),
        "present relations array replaces the set (drops the prior target)"
    );
}

/// ORB-10253: an empty `relations` array on `PATCH /api/tasks/:id` clears the
/// relation set, while an absent field leaves it unchanged (covered implicitly
/// by every other PATCH test that omits `relations`).
#[tokio::test]
async fn update_task_clears_relations_with_empty_array() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let target = seed_backlog_task(&runtime, "Relation target to be cleared");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Task whose relations get cleared".to_string(),
            description: "empty array clears the relation set".to_string(),
            status: Some(TaskStatus::Backlog),
            workspace_path: Some(".".to_string()),
            relations: vec![TaskRelation {
                relation_type: TaskRelationType::RelatedTo,
                target: target.id.clone(),
            }],
            ..Default::default()
        })
        .expect("seed task with relation");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(runtime)));

    let response = app
        .oneshot(patch_json(
            &format!("/tasks/{}", task.id),
            json!({ "relations": [] }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["relations"], json!([]), "empty array clears relations");
}

/// ORB-10253: an invalid relation target must surface Orbit's own validation
/// error as a 4xx — never a silent drop. A malformed target fails
/// `validate_task_relations_for_source` and reaches the client through
/// `map_runtime_error` as a 400.
#[tokio::test]
async fn create_task_rejects_invalid_relation_target() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let app = router().with_state(crate::state::DashboardState::single(Arc::new(runtime)));

    let response = app
        .oneshot(post_json(
            "/tasks",
            json!({
                "title": "task with a bad relation",
                "description": "malformed relation target must be rejected",
                "status": "backlog",
                "relations": [{ "type": "related_to", "target": "not-a-task-id" }],
            }),
        ))
        .await
        .expect("response");

    assert!(
        response.status().is_client_error(),
        "invalid relation target must be a 4xx, got {}",
        response.status()
    );
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("not-a-task-id")),
        "error must carry Orbit's validation text: {body}"
    );
}
