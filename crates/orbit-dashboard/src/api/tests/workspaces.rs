//! Tests for the global (cross-workspace) endpoints (ORB-00030).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use orbit_core::command::task::TaskAddParams;
use orbit_core::{ActorIdentity, OrbitRuntime, TaskStatus};
use serde_json::json;
use tower::ServiceExt;

use super::super::*;
use super::test_support::body_json;
use crate::state::{DashboardState, WsEntry};

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Create an on-disk workspace under `base/<name>`, seed one in-progress task,
/// and return `(orbit_dir, repo_root)`. The workspace persists after the
/// runtime is dropped, so global mode can reopen it via `from_roots`.
fn seed_workspace(global_root: &Path, base: &Path, name: &str) -> (PathBuf, PathBuf) {
    let repo_root = base.join(name);
    let orbit_dir = repo_root.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create .orbit");
    std::fs::write(orbit_dir.join("config.toml"), "").expect("write config");
    let runtime = OrbitRuntime::from_roots(global_root, &orbit_dir)
        .expect("build runtime")
        .with_actor(ActorIdentity::human("human"));
    runtime
        .add_task(TaskAddParams {
            title: format!("{name} task"),
            description: "seed".to_string(),
            workspace_path: Some(".".to_string()),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("add task");
    (orbit_dir, repo_root)
}

#[tokio::test]
async fn workspaces_endpoint_reports_single_default() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let state = DashboardState::single(Arc::new(runtime));
    let response = router()
        .with_state(state)
        .oneshot(get("/workspaces"))
        .await
        .expect("response");
    let body = body_json(response).await;
    let entries = body.as_array().expect("array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], json!("default"));
    assert_eq!(entries[0]["is_default"], json!(true));
    assert_eq!(entries[0]["status"], json!("active"));
}

#[tokio::test]
async fn tasks_all_in_single_mode_tags_default_workspace() {
    let runtime = OrbitRuntime::in_memory()
        .expect("build runtime")
        .with_actor(ActorIdentity::human("human"));
    runtime
        .add_task(TaskAddParams {
            title: "solo".to_string(),
            description: "seed".to_string(),
            workspace_path: Some(".".to_string()),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("add task");
    let state = DashboardState::single(Arc::new(runtime));
    let response = router()
        .with_state(state)
        .oneshot(get("/tasks/all"))
        .await
        .expect("response");
    let body = body_json(response).await;
    let tasks = body.as_array().expect("array");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["workspace_id"], json!("default"));
    assert_eq!(tasks[0]["workspace_name"], json!("default"));
}

#[tokio::test]
async fn tasks_all_aggregates_active_workspaces_and_skips_inactive() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");

    let (alpha_orbit, alpha_repo) = seed_workspace(&global_root, tmp.path(), "alpha");
    let (beta_orbit, beta_repo) = seed_workspace(&global_root, tmp.path(), "beta");

    let entries = vec![
        WsEntry {
            id: "alpha".to_string(),
            name: "alpha".to_string(),
            repo_root: alpha_repo,
            orbit_dir: alpha_orbit,
            active: true,
        },
        WsEntry {
            id: "beta".to_string(),
            name: "beta".to_string(),
            repo_root: beta_repo,
            orbit_dir: beta_orbit,
            active: true,
        },
        WsEntry {
            id: "gone".to_string(),
            name: "gone".to_string(),
            repo_root: tmp.path().join("missing"),
            orbit_dir: tmp.path().join("missing/.orbit"),
            active: false,
        },
    ];
    let state = DashboardState::global(global_root, entries, Some("alpha".to_string()));

    // Aggregate task list: one task per active workspace, tagged; none from the
    // inactive workspace.
    let response = router()
        .with_state(state.clone())
        .oneshot(get("/tasks/all"))
        .await
        .expect("response");
    let body = body_json(response).await;
    let tasks = body.as_array().expect("array");
    assert_eq!(tasks.len(), 2);
    let ws_ids: HashSet<&str> = tasks
        .iter()
        .map(|t| t["workspace_id"].as_str().expect("workspace_id"))
        .collect();
    assert_eq!(ws_ids, HashSet::from(["alpha", "beta"]));
    assert!(tasks.iter().all(|t| t["workspace_name"].is_string()));
    // ORB-00037: every aggregate task carries its workspace's filesystem path.
    assert!(tasks.iter().all(|t| {
        t["workspace_root"]
            .as_str()
            .is_some_and(|root| root.ends_with(t["workspace_name"].as_str().expect("name")))
    }));

    // Workspace listing: all three, with status + default flag.
    let response = router()
        .with_state(state)
        .oneshot(get("/workspaces"))
        .await
        .expect("response");
    let body = body_json(response).await;
    let listed = body.as_array().expect("array");
    assert_eq!(listed.len(), 3);
    let gone = listed
        .iter()
        .find(|w| w["id"] == json!("gone"))
        .expect("gone entry");
    assert_eq!(gone["status"], json!("invalid"));
    let alpha = listed
        .iter()
        .find(|w| w["id"] == json!("alpha"))
        .expect("alpha entry");
    assert_eq!(alpha["is_default"], json!(true));
    assert_eq!(alpha["status"], json!("active"));
    // ORB-00037: entries expose the filesystem path (repo root + orbit_dir).
    assert!(
        alpha["root"]
            .as_str()
            .is_some_and(|root| root.ends_with("alpha"))
    );
    assert!(
        alpha["orbit_dir"]
            .as_str()
            .is_some_and(|dir| dir.ends_with(".orbit"))
    );
}

/// ORB-10008: workspace selection failures surface over HTTP as clean 4xx
/// JSON bodies (unknown -> 404, inactive -> 400, no default -> 400), never a
/// 500 or a panic.
#[tokio::test]
async fn workspace_selection_errors_are_clean_4xx_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");
    let (orbit_dir, repo_root) = seed_workspace(&global_root, tmp.path(), "alpha");
    let entries = vec![
        WsEntry {
            id: "alpha".to_string(),
            name: "alpha".to_string(),
            repo_root,
            orbit_dir,
            active: true,
        },
        WsEntry {
            id: "stale".to_string(),
            name: "stale".to_string(),
            repo_root: tmp.path().join("missing"),
            orbit_dir: tmp.path().join("missing/.orbit"),
            active: false,
        },
    ];
    // No default workspace configured: requests must select one explicitly.
    let state = DashboardState::global(global_root, entries, None);

    // Unknown workspace id -> 404 with a JSON error body.
    let response = router()
        .with_state(state.clone())
        .oneshot(get("/tasks?workspace=ghost"))
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("unknown workspace: ghost"))
    );

    // Inactive (stale-path) workspace -> 400, never built.
    let response = router()
        .with_state(state.clone())
        .oneshot(get("/tasks?workspace=stale"))
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("inactive"))
    );

    // No selection and no default -> 400 telling the caller what to pass.
    let response = router()
        .with_state(state.clone())
        .oneshot(get("/tasks"))
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("workspace"))
    );

    // A valid selection on the same state still works.
    let response = router()
        .with_state(state)
        .oneshot(get("/tasks?workspace=alpha"))
        .await
        .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}

/// ORB-10291: a task in one registered workspace depending on a task in
/// another workspace sharing the same global root must resolve that
/// dependency's real status, not `[missing]`. Dependency resolution has to
/// use the coordination registry's global status projection
/// (`OrbitRuntime::task_status_index`) rather than the selected workspace's
/// own task list.
#[tokio::test]
async fn cross_workspace_dependency_resolves_global_status_not_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");

    let (beta_orbit, beta_repo) = seed_workspace(&global_root, tmp.path(), "beta");
    let beta_task = {
        let beta_runtime = OrbitRuntime::from_roots(&global_root, &beta_orbit)
            .expect("build beta runtime")
            .with_actor(ActorIdentity::human("human"));
        beta_runtime
            .add_task(TaskAddParams {
                title: "beta dependency".to_string(),
                description: "seed".to_string(),
                workspace_path: Some(".".to_string()),
                status: Some(TaskStatus::Done),
                ..Default::default()
            })
            .expect("add beta task")
    };

    let alpha_repo = tmp.path().join("alpha");
    let alpha_orbit = alpha_repo.join(".orbit");
    std::fs::create_dir_all(&alpha_orbit).expect("create .orbit");
    std::fs::write(alpha_orbit.join("config.toml"), "").expect("write config");
    let alpha_task = {
        let alpha_runtime = OrbitRuntime::from_roots(&global_root, &alpha_orbit)
            .expect("build alpha runtime")
            .with_actor(ActorIdentity::human("human"));
        alpha_runtime
            .add_task(TaskAddParams {
                title: "alpha dependent".to_string(),
                description: "seed".to_string(),
                workspace_path: Some(".".to_string()),
                status: Some(TaskStatus::InProgress),
                dependencies: vec![beta_task.id.clone()],
                ..Default::default()
            })
            .expect("add alpha task")
    };

    let entries = vec![
        WsEntry {
            id: "alpha".to_string(),
            name: "alpha".to_string(),
            repo_root: alpha_repo,
            orbit_dir: alpha_orbit,
            active: true,
        },
        WsEntry {
            id: "beta".to_string(),
            name: "beta".to_string(),
            repo_root: beta_repo,
            orbit_dir: beta_orbit,
            active: true,
        },
    ];
    let state = DashboardState::global(global_root, entries, Some("alpha".to_string()));
    let expected_label = format!("{} [done]", beta_task.id);

    // GET /tasks?workspace=alpha: the cross-workspace dependency resolves to
    // beta's real status, and the response contains only alpha's own task.
    let response = router()
        .with_state(state.clone())
        .oneshot(get("/tasks?workspace=alpha"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let tasks = body.as_array().expect("array");
    assert_eq!(
        tasks.len(),
        1,
        "workspace-local list must contain only alpha's own task, got {tasks:?}"
    );
    let alpha_row = &tasks[0];
    assert_eq!(alpha_row["id"], json!(alpha_task.id));
    let labels: Vec<&str> = alpha_row["resolved_dependencies"]
        .as_array()
        .expect("resolved_dependencies array")
        .iter()
        .map(|value| value.as_str().expect("dependency label"))
        .collect();
    assert_eq!(labels, vec![expected_label.as_str()]);

    // GET /tasks/<alpha-id>?workspace=alpha: the show projection reports the
    // identical cross-workspace dependency status.
    let response = router()
        .with_state(state)
        .oneshot(get(&format!("/tasks/{}?workspace=alpha", alpha_task.id)))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let labels: Vec<&str> = body["resolved_dependencies"]
        .as_array()
        .expect("resolved_dependencies array")
        .iter()
        .map(|value| value.as_str().expect("dependency label"))
        .collect();
    assert_eq!(labels, vec![expected_label.as_str()]);
}

/// ORB-00037: the pure display helper collapses `$HOME` to `~` and otherwise
/// renders paths verbatim.
#[test]
fn abbreviate_home_collapses_home_prefix() {
    use super::super::workspaces::abbreviate_home;

    let home = PathBuf::from("/home/dan");
    assert_eq!(
        abbreviate_home(&PathBuf::from("/home/dan/ws/orbit"), Some(&home)),
        "~/ws/orbit"
    );
    // The home directory itself collapses to a bare `~`.
    assert_eq!(abbreviate_home(&home, Some(&home)), "~");
    // Paths outside home are untouched.
    assert_eq!(
        abbreviate_home(&PathBuf::from("/srv/data"), Some(&home)),
        "/srv/data"
    );
    // A sibling that merely shares a name prefix is not under home.
    assert_eq!(
        abbreviate_home(&PathBuf::from("/home/danish/x"), Some(&home)),
        "/home/danish/x"
    );
    // No home configured => render verbatim.
    assert_eq!(
        abbreviate_home(&PathBuf::from("/home/dan/ws"), None),
        "/home/dan/ws"
    );
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::ORIGIN, "http://localhost:7878")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// Titles of the dashboard task list for one workspace, via the HTTP surface.
async fn task_titles(state: &DashboardState, workspace: &str) -> Vec<String> {
    let response = router()
        .with_state(state.clone())
        .oneshot(get(&format!("/tasks?workspace={workspace}")))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    body_json(response)
        .await
        .as_array()
        .expect("array")
        .iter()
        .map(|t| t["title"].as_str().expect("title").to_string())
        .collect()
}

/// ORB-00042: `POST /tasks?workspace=<id>` creates the task in *that*
/// workspace, not the server's default.
#[tokio::test]
async fn create_task_with_workspace_param_binds_to_that_workspace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");
    let (alpha_orbit, alpha_repo) = seed_workspace(&global_root, tmp.path(), "alpha");
    let (beta_orbit, beta_repo) = seed_workspace(&global_root, tmp.path(), "beta");
    let entries = vec![
        WsEntry {
            id: "alpha".to_string(),
            name: "alpha".to_string(),
            repo_root: alpha_repo,
            orbit_dir: alpha_orbit,
            active: true,
        },
        WsEntry {
            id: "beta".to_string(),
            name: "beta".to_string(),
            repo_root: beta_repo,
            orbit_dir: beta_orbit,
            active: true,
        },
    ];
    // alpha is the default: an unrouted create would land there.
    let state = DashboardState::global(global_root, entries, Some("alpha".to_string()));

    let response = router()
        .with_state(state.clone())
        .oneshot(post_json(
            "/tasks?workspace=beta",
            json!({ "title": "routed to beta", "description": "ORB-00042" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let created = body_json(response).await;
    assert!(created["id"].is_string(), "create returns the task");

    let beta_titles = task_titles(&state, "beta").await;
    assert!(
        beta_titles.iter().any(|t| t == "routed to beta"),
        "task must live in the selected workspace, got {beta_titles:?}"
    );
    let alpha_titles = task_titles(&state, "alpha").await;
    assert!(
        !alpha_titles.iter().any(|t| t == "routed to beta"),
        "task must not leak into the default workspace, got {alpha_titles:?}"
    );

    // Omitting `?workspace=` falls back to the configured default (alpha).
    let response = router()
        .with_state(state.clone())
        .oneshot(post_json(
            "/tasks",
            json!({ "title": "defaulted to alpha", "description": "ORB-00042" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let alpha_titles = task_titles(&state, "alpha").await;
    assert!(
        alpha_titles.iter().any(|t| t == "defaulted to alpha"),
        "omitted selector must use the default workspace, got {alpha_titles:?}"
    );
}

/// ORB-00042: creating against an unknown workspace is a clean 404 and no
/// task is created anywhere — never a silent fallback to the default.
#[tokio::test]
async fn create_task_with_unknown_workspace_is_404_and_creates_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global_root = tmp.path().join("global");
    std::fs::create_dir_all(&global_root).expect("create global root");
    let (alpha_orbit, alpha_repo) = seed_workspace(&global_root, tmp.path(), "alpha");
    let entries = vec![WsEntry {
        id: "alpha".to_string(),
        name: "alpha".to_string(),
        repo_root: alpha_repo,
        orbit_dir: alpha_orbit,
        active: true,
    }];
    let state = DashboardState::global(global_root, entries, Some("alpha".to_string()));

    let response = router()
        .with_state(state.clone())
        .oneshot(post_json(
            "/tasks?workspace=ghost",
            json!({ "title": "lost", "description": "should not exist" }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("unknown workspace: ghost"))
    );

    let alpha_titles = task_titles(&state, "alpha").await;
    assert!(
        !alpha_titles.iter().any(|t| t == "lost"),
        "nothing may be created in the default workspace, got {alpha_titles:?}"
    );
}
