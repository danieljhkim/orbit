//! Tests for the global (cross-workspace) endpoints (ORB-00030).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request};
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
}
