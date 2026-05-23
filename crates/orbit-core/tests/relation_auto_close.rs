#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use orbit_common::types::{FrictionStatus, TaskStatus};
use orbit_core::OrbitRuntime;
use orbit_engine::{TaskAutomationUpdate, TaskWriteHost};
use orbit_store::friction_store::{
    FrictionAddParams, add_friction, resolve_friction_by_task, show_friction,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn test_runtime() -> (TempDir, OrbitRuntime, std::path::PathBuf) {
    let root = TempDir::new().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, repo_root)
}

fn add_test_friction(runtime: &OrbitRuntime) -> String {
    let stored = add_friction(
        &runtime.data_root().join("frictions"),
        FrictionAddParams {
            model: "codex".to_string(),
            body: "Approval should close this friction".to_string(),
            tags: vec!["tooling".to_string()],
            during_task: None,
            created_at: Utc.with_ymd_and_hms(2026, 5, 17, 4, 5, 0).unwrap(),
        },
    )
    .expect("add friction");
    stored.record.id
}

fn add_task_with_resolves(
    runtime: &OrbitRuntime,
    repo_root: &std::path::Path,
    target: &str,
    status: &str,
) -> String {
    let task = runtime
        .run_tool(
            "orbit.task.add",
            json!({
                "title": format!("Resolve {target}"),
                "description": "Fixture task with a resolves relation.",
                "acceptance_criteria": ["Relation is visible."],
                "workspace": repo_root.to_string_lossy(),
                "type": "feature",
                "relations": [
                    { "type": "resolves", "target": target }
                ],
                "model": "codex"
            }),
        )
        .expect("add task");
    let task_id = task["id"].as_str().expect("task id").to_string();
    // ORB-00255 retired `plan` and `status` from the orbit.task.add schema,
    // so seed the plan via update and approve into the desired status here.
    runtime
        .run_tool(
            "orbit.task.update",
            json!({
                "id": task_id,
                "plan": "1. Exercise the relation transition.",
                "model": "codex",
            }),
        )
        .expect("set plan via update");
    if status == "backlog" {
        runtime
            .run_tool(
                "orbit.task.approve",
                json!({ "id": task_id, "model": "codex" }),
            )
            .expect("approve task into backlog");
    }
    task_id
}

fn move_backlog_task_to_review(runtime: &OrbitRuntime, task_id: &str) {
    runtime
        .run_tool(
            "orbit.task.start",
            json!({ "id": task_id, "model": "codex" }),
        )
        .expect("start task");
    runtime
        .run_tool(
            "orbit.task.update",
            json!({
                "id": task_id,
                "status": "review",
                "execution_summary": "Ready for approval.",
                "model": "codex"
            }),
        )
        .expect("move task to review");
}

fn assert_friction_resolved_by(runtime: &OrbitRuntime, friction_id: &str, task_id: &str) {
    let friction = runtime
        .run_tool("orbit.friction.show", json!({ "id": friction_id }))
        .expect("show friction");
    assert_eq!(friction["status"], json!("resolved"));
    assert!(friction["resolved_at"].as_str().is_some());
    assert_eq!(friction["resolved_by_task"], json!(task_id));
}

#[test]
fn review_approval_resolves_related_friction_and_surfaces_json_fields() {
    let (_root, runtime, repo_root) = test_runtime();
    let friction_id = add_test_friction(&runtime);
    let task_id = add_task_with_resolves(&runtime, &repo_root, &friction_id, "backlog");
    move_backlog_task_to_review(&runtime, &task_id);

    runtime
        .run_tool(
            "orbit.task.approve",
            json!({ "id": task_id, "model": "codex" }),
        )
        .expect("approve task");

    assert_friction_resolved_by(&runtime, &friction_id, &task_id);

    let task = runtime
        .run_tool("orbit.task.show", json!({ "id": task_id }))
        .expect("show task");
    assert_eq!(task["relations"][0]["type"], json!("resolves"));
    assert_eq!(task["relations"][0]["target"], json!(friction_id));
}

#[test]
fn task_update_to_done_resolves_related_friction() {
    let (_root, runtime, repo_root) = test_runtime();
    let friction_id = add_test_friction(&runtime);
    let task_id = add_task_with_resolves(&runtime, &repo_root, &friction_id, "backlog");

    let updated = runtime
        .run_tool(
            "orbit.task.update",
            json!({
                "id": task_id,
                "status": "done",
                "model": "codex"
            }),
        )
        .expect("update task to done");
    assert_eq!(updated["status"], json!("done"));

    assert_friction_resolved_by(&runtime, &friction_id, &task_id);
}

#[test]
fn automation_update_to_done_resolves_related_friction() {
    let (_root, runtime, repo_root) = test_runtime();
    let friction_id = add_test_friction(&runtime);
    let task_id = add_task_with_resolves(&runtime, &repo_root, &friction_id, "backlog");

    runtime
        .apply_task_automation_update(
            &task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::Done),
                ..TaskAutomationUpdate::default()
            },
        )
        .expect("automation update task to done");

    assert_friction_resolved_by(&runtime, &friction_id, &task_id);
}

#[test]
fn approving_task_with_dangling_friction_relation_records_event_but_succeeds() {
    let (_root, runtime, repo_root) = test_runtime();
    let task_id = add_task_with_resolves(&runtime, &repo_root, "F9999-12-999", "backlog");
    move_backlog_task_to_review(&runtime, &task_id);

    let approved = runtime
        .run_tool(
            "orbit.task.approve",
            json!({ "id": task_id, "model": "codex" }),
        )
        .expect("approve task");
    assert_eq!(approved["status"], json!("done"));

    let events = runtime.list_session_events(20).expect("session events");
    assert!(events.iter().any(|event| {
        event.payload.get("type") == Some(&Value::String("TaskRelationDangling".to_string()))
            && event
                .payload
                .get("data")
                .and_then(Value::as_object)
                .is_some_and(|data| {
                    data.get("task_id") == Some(&Value::String(task_id.clone()))
                        && data.get("target") == Some(&Value::String("F9999-12-999".to_string()))
                })
    }));
    assert!(
        !runtime
            .data_root()
            .join("frictions")
            .join("9999-12")
            .join("F999.md")
            .exists()
    );
}

#[test]
fn approving_task_does_not_overwrite_existing_friction_resolution() {
    let (_root, runtime, repo_root) = test_runtime();
    let friction_id = add_test_friction(&runtime);
    let original_resolved_at = Utc.with_ymd_and_hms(2026, 5, 17, 3, 0, 0).unwrap();
    resolve_friction_by_task(
        &runtime.data_root().join("frictions"),
        &friction_id,
        "ORB-99999",
        original_resolved_at,
    )
    .expect("pre-resolve friction");

    let task_id = add_task_with_resolves(&runtime, &repo_root, &friction_id, "backlog");
    move_backlog_task_to_review(&runtime, &task_id);
    runtime
        .run_tool(
            "orbit.task.approve",
            json!({ "id": task_id, "model": "codex" }),
        )
        .expect("approve task");

    let stored = show_friction(&runtime.data_root().join("frictions"), &friction_id)
        .expect("show friction")
        .expect("friction exists");
    assert_eq!(stored.record.status, FrictionStatus::Resolved);
    assert_eq!(stored.record.resolved_by_task.as_deref(), Some("ORB-99999"));
    assert_eq!(stored.record.resolved_at, Some(original_resolved_at));
}

#[test]
fn approving_proposed_task_does_not_resolve_friction() {
    let (_root, runtime, repo_root) = test_runtime();
    let friction_id = add_test_friction(&runtime);
    let task_id = add_task_with_resolves(&runtime, &repo_root, &friction_id, "proposed");

    let approved = runtime
        .run_tool(
            "orbit.task.approve",
            json!({ "id": task_id, "model": "codex" }),
        )
        .expect("approve proposed task");
    assert_eq!(approved["status"], json!("backlog"));

    let stored = show_friction(&runtime.data_root().join("frictions"), &friction_id)
        .expect("show friction")
        .expect("friction exists");
    assert_eq!(stored.record.status, FrictionStatus::Open);
    assert_eq!(stored.record.resolved_at, None);
    assert_eq!(stored.record.resolved_by_task, None);

    let task = runtime.get_task(&task_id).expect("get task");
    assert_eq!(task.status, TaskStatus::Backlog);
}
