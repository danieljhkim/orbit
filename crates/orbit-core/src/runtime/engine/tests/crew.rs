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
[crews.primary]
planner = { model = "default-planner", provider = "codex", backend = "cli" }
implementer = { model = "default-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "default-reviewer", provider = "codex", backend = "cli" }

[crews.beta]
planner = { model = "beta-planner", provider = "codex", backend = "cli" }
implementer = { model = "beta-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "beta-reviewer", provider = "codex", backend = "cli" }

[crews.gamma]
planner = { model = "gamma-planner", provider = "codex", backend = "cli" }
implementer = { model = "gamma-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "gamma-reviewer", provider = "codex", backend = "cli" }

[workflow]
default_crew = "primary"
"#,
    )
    .expect("write test config");
    let runtime = OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
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
    let task_id = add_task_with_crew(&runtime, "beta");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({ "task_ids": [task_id] }))
        .expect("resolve crew");

    assert_eq!(crew.name, "beta");
    assert_eq!(crew.planner.model, "beta-planner");
    assert_eq!(crew.implementer.model, "beta-implementer");
    assert_eq!(crew.reviewer.model, "beta-reviewer");
}

#[test]
fn record_run_crew_persists_singleton_task_ids_task_crew_models() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "beta");
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

    assert_eq!(crew.name, "beta");
    assert_eq!(stored.resolved_crew.as_deref(), Some("beta"));
    assert_eq!(stored.planner_model.as_deref(), Some("beta-planner"));
    assert_eq!(
        stored.implementer_model.as_deref(),
        Some("beta-implementer")
    );
    assert_eq!(stored.reviewer_model.as_deref(), Some("beta-reviewer"));
}

#[test]
fn explicit_crew_override_wins_over_singleton_task_ids_task_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "beta");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "crew": "gamma",
            "task_ids": [task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "gamma");
    assert_eq!(crew.implementer.model, "gamma-implementer");
}

#[test]
fn multi_task_ids_without_override_falls_back_to_default_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let beta_task_id = add_task_with_crew(&runtime, "beta");
    let gamma_task_id = add_task_with_crew(&runtime, "gamma");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "task_ids": [beta_task_id, gamma_task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "primary");
    assert_eq!(crew.implementer.model, "default-implementer");
}
