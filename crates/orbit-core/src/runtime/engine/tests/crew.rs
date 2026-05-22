//! Sibling tests for `crew.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use chrono::Utc;
use serde_json::json;
use tempfile::{TempDir, tempdir};

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

fn runtime_with_named_crews() -> (TempDir, OrbitRuntime) {
    let root = tempdir().expect("create temp root");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[crews.opus-codex]
planner = { model = "default-planner", provider = "codex", backend = "cli" }
implementer = { model = "default-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "default-reviewer", provider = "codex", backend = "cli" }

[crews.silver]
planner = { model = "silver-planner", provider = "codex", backend = "cli" }
implementer = { model = "silver-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "silver-reviewer", provider = "codex", backend = "cli" }

[crews.bronze]
planner = { model = "bronze-planner", provider = "codex", backend = "cli" }
implementer = { model = "bronze-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "bronze-reviewer", provider = "codex", backend = "cli" }

[workflow]
default_crew = "opus-codex"
"#,
    )
    .expect("write test config");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
    (root, runtime)
}

fn add_task_with_crew(runtime: &OrbitRuntime, crew: &str) -> String {
    runtime
        .add_task(TaskAddParams {
            title: format!("{crew} task"),
            description: "Task fixture for crew resolution.".to_string(),
            crew: Some(crew.to_string()),
            ..Default::default()
        })
        .expect("add task")
        .id
}

#[test]
fn run_input_task_ids_singleton_resolves_task_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "silver");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({ "task_ids": [task_id] }))
        .expect("resolve crew");

    assert_eq!(crew.name, "silver");
    assert_eq!(crew.planner.model, "silver-planner");
    assert_eq!(crew.implementer.model, "silver-implementer");
    assert_eq!(crew.reviewer.model, "silver-reviewer");
}

#[test]
fn record_run_crew_persists_singleton_task_ids_task_crew_models() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "silver");
    let input = json!({ "task_ids": [task_id] });
    let run = runtime
        .stores()
        .jobs()
        .insert_run("agent_implement", 1, Utc::now(), Some(input.clone()), None)
        .expect("insert run");

    let crew = runtime
        .record_run_crew_from_input(&run.run_id, &input)
        .expect("record crew");
    let stored = runtime.show_job_run(&run.run_id).expect("show stored run");

    assert_eq!(crew.name, "silver");
    assert_eq!(stored.resolved_crew.as_deref(), Some("silver"));
    assert_eq!(stored.planner_model.as_deref(), Some("silver-planner"));
    assert_eq!(
        stored.implementer_model.as_deref(),
        Some("silver-implementer")
    );
    assert_eq!(stored.reviewer_model.as_deref(), Some("silver-reviewer"));
}

#[test]
fn explicit_crew_override_wins_over_singleton_task_ids_task_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "silver");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "crew": "bronze",
            "task_ids": [task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "bronze");
    assert_eq!(crew.implementer.model, "bronze-implementer");
}

#[test]
fn multi_task_ids_without_override_falls_back_to_default_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let silver_task_id = add_task_with_crew(&runtime, "silver");
    let bronze_task_id = add_task_with_crew(&runtime, "bronze");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "task_ids": [silver_task_id, bronze_task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "opus-codex");
    assert_eq!(crew.implementer.model, "default-implementer");
}
