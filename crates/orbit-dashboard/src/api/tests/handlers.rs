use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::command::task::TaskAddParams;
use orbit_core::{JobRunState, OrbitRuntime, TaskStatus};
use serde_json::json;
use tower::ServiceExt;

use super::super::*;
use super::test_support::{body_json, seed_run};

async fn request_cancel(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/runs/{run_id}/cancel"));
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    router()
        .with_state(Arc::new(runtime))
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn request_tasks(runtime: OrbitRuntime) -> Response {
    router()
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/tasks")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn patch_task_crew(runtime: OrbitRuntime, task_id: &str, crew: &str) -> Response {
    router()
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/tasks/{task_id}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:7878")
                .body(Body::from(format!(r#"{{"crew":"{crew}"}}"#)))
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn request_crews(runtime: OrbitRuntime) -> Response {
    router()
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/crews")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

fn runtime_with_custom_crews() -> (tempfile::TempDir, OrbitRuntime) {
    let root = tempfile::tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[crews.beta]
planner = { model = "claude-beta-plan", provider = "claude", backend = "cli" }
implementer = { model = "codex-beta-impl", provider = "codex", backend = "cli" }
reviewer = { model = "codex-beta-review", provider = "codex", backend = "cli" }

[crews.alpha]
planner = { model = "alpha-plan-model", provider = "claude", backend = "cli" }
implementer = { model = "alpha-impl-model", provider = "codex", backend = "cli" }
reviewer = { model = "alpha-review-model", provider = "codex", backend = "cli" }

[workflow]
default_crew = "beta"
"#,
    )
    .expect("write config");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime)
}

fn runtime_with_stale_task_crew() -> (tempfile::TempDir, OrbitRuntime, String) {
    let root = tempfile::tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[crews.beta]
planner = { model = "claude-beta-plan", provider = "claude", backend = "cli" }
implementer = { model = "codex-beta-impl", provider = "codex", backend = "cli" }
reviewer = { model = "codex-beta-review", provider = "codex", backend = "cli" }

[crews.all-codex]
planner = { model = "legacy-plan-model", provider = "codex", backend = "cli" }
implementer = { model = "legacy-impl-model", provider = "codex", backend = "cli" }
reviewer = { model = "legacy-review-model", provider = "codex", backend = "cli" }

[workflow]
default_crew = "beta"
"#,
    )
    .expect("write initial config");
    let initial_runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build initial runtime");
    let task = initial_runtime
        .add_task(TaskAddParams {
            title: "Stale crew task".to_string(),
            description: "Fixture with an explicit crew removed from config.".to_string(),
            status: Some(TaskStatus::Backlog),
            crew: Some("all-codex".to_string()),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("create stale crew task");

    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[crews.beta]
planner = { model = "claude-beta-plan", provider = "claude", backend = "cli" }
implementer = { model = "codex-beta-impl", provider = "codex", backend = "cli" }
reviewer = { model = "codex-beta-review", provider = "codex", backend = "cli" }

[workflow]
default_crew = "beta"
"#,
    )
    .expect("write reduced config");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build reduced runtime");
    (root, runtime, task.id)
}

fn seed_task(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    dependencies: Vec<String>,
) -> orbit_core::Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture for {title}."),
            status: Some(status),
            dependencies,
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("create task")
}

#[tokio::test]
async fn tasks_with_stale_explicit_crew_fall_back_to_default_projection() {
    let (_root, runtime, task_id) = runtime_with_stale_task_crew();

    let response = request_tasks(runtime.clone()).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("tasks response array");
    let task = rows
        .iter()
        .find(|task| task["id"].as_str() == Some(task_id.as_str()))
        .expect("stale crew task is listed");
    assert_eq!(task["crew"], json!("all-codex"));
    assert_eq!(task["resolved_crew"], json!("beta"));
    assert_eq!(task["planner_model"], json!("claude-beta-plan"));
    assert_eq!(task["implementer_model"], json!("codex-beta-impl"));
    assert_eq!(task["reviewer_model"], json!("codex-beta-review"));

    let response = patch_task_crew(runtime, &task_id, "all-codex").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn crews_endpoint_returns_sorted_runtime_registry() {
    let (_root, runtime) = runtime_with_custom_crews();

    let response = request_crews(runtime).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["default_crew"], json!("beta"));
    let crews = body["crews"].as_array().expect("crews array");
    assert_eq!(crews.len(), 2);
    assert_eq!(crews[0]["name"], json!("alpha"));
    assert_eq!(crews[0]["is_default"], json!(false));
    assert_eq!(crews[0]["planner_model"], json!("alpha-plan-model"));
    assert_eq!(crews[0]["implementer_model"], json!("alpha-impl-model"));
    assert_eq!(crews[0]["reviewer_model"], json!("alpha-review-model"));
    assert_eq!(crews[1]["name"], json!("beta"));
    assert_eq!(crews[1]["is_default"], json!(true));
    assert_eq!(crews[1]["planner_model"], json!("claude-beta-plan"));
    assert_eq!(crews[1]["implementer_model"], json!("codex-beta-impl"));
    assert_eq!(crews[1]["reviewer_model"], json!("codex-beta-review"));
}

#[tokio::test]
async fn require_localhost_origin_rejects_prefix_match() {
    let cases = [
        ("http://localhost.evil.com", "localhost prefix"),
        ("http://127.0.0.1.evil.com", "127.0.0.1 prefix"),
    ];

    for (index, (origin, label)) in cases.into_iter().enumerate() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let run = seed_run(
            &runtime,
            &format!("jrun-web-cancel-prefix-{index}"),
            "web_cancel_prefix",
            JobRunState::Pending,
        );

        let response = request_cancel(runtime.clone(), &run.run_id, Some(origin)).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
        let stored = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(stored.state, JobRunState::Pending, "{label}");
    }
}

#[tokio::test]
async fn tasks_resolve_dependency_statuses_from_all_tasks() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let done = seed_task(
        &runtime,
        "Completed dependency",
        TaskStatus::Done,
        Vec::new(),
    );
    let archived = seed_task(
        &runtime,
        "Archived dependency",
        TaskStatus::Backlog,
        Vec::new(),
    );
    runtime.archive_task(&archived.id).expect("archive task");
    let rejected = seed_task(
        &runtime,
        "Rejected dependency",
        TaskStatus::Rejected,
        Vec::new(),
    );
    let visible = seed_task(
        &runtime,
        "Visible dependent",
        TaskStatus::Backlog,
        vec![done.id.clone(), archived.id.clone(), rejected.id.clone()],
    );

    let response = request_tasks(runtime).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows = body.as_array().expect("tasks response array");
    assert!(
        rows.iter()
            .any(|task| task["id"].as_str() == Some(&visible.id))
    );
    assert!(
        rows.iter()
            .any(|task| task["id"].as_str() == Some(&rejected.id))
    );
    assert!(
        !rows
            .iter()
            .any(|task| task["id"].as_str() == Some(&done.id))
    );
    assert!(
        !rows
            .iter()
            .any(|task| task["id"].as_str() == Some(&archived.id))
    );

    let visible_json = rows
        .iter()
        .find(|task| task["id"].as_str() == Some(&visible.id))
        .expect("visible task row");
    let dependencies = visible_json["resolved_dependencies"]
        .as_array()
        .expect("resolved dependencies array");
    let dependency_labels = dependencies
        .iter()
        .map(|value| value.as_str().expect("dependency label"))
        .collect::<Vec<_>>();
    let done_label = format!("{} [done]", done.id);
    let archived_label = format!("{} [archived]", archived.id);
    let rejected_label = format!("{} [rejected]", rejected.id);
    assert!(dependency_labels.contains(&done_label.as_str()));
    assert!(dependency_labels.contains(&archived_label.as_str()));
    assert!(dependency_labels.contains(&rejected_label.as_str()));
}

#[tokio::test]
async fn require_localhost_origin_rejects_https_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let run = seed_run(
        &runtime,
        "jrun-web-cancel-https-origin",
        "web_cancel_https_origin",
        JobRunState::Pending,
    );

    let response = request_cancel(runtime.clone(), &run.run_id, Some("https://localhost")).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(stored.state, JobRunState::Pending);
}

#[tokio::test]
async fn require_localhost_origin_accepts_localhost_with_port() {
    let cases = [
        ("http://localhost:7878", "localhost"),
        ("http://127.0.0.1:7878", "127-0-0-1"),
    ];

    for (origin, label) in cases {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let run = seed_run(
            &runtime,
            &format!("jrun-web-cancel-origin-port-{label}"),
            "web_cancel_origin_port",
            JobRunState::Pending,
        );

        let response = request_cancel(runtime.clone(), &run.run_id, Some(origin)).await;

        assert_eq!(response.status(), StatusCode::OK, "{label}");
        let stored = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(stored.state, JobRunState::Cancelled, "{label}");
    }
}

#[tokio::test]
async fn require_localhost_origin_blocks_cross_origin_get_with_attacker_origin() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let response = router()
        .with_state(Arc::new(runtime))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/tasks")
                .header(header::ORIGIN, "http://localhost.evil.com")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
