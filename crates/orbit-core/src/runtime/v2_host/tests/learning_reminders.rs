use orbit_common::types::{LearningInjectionCaps, LearningScope, Task};
use orbit_engine::RuntimeHost;
use orbit_store::{LearningCreateParams, LearningSearchParams};
use serde_json::json;

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

fn create_learning(
    runtime: &OrbitRuntime,
    summary: &str,
    paths: &[&str],
    tags: &[&str],
    priority: Option<u8>,
) -> orbit_common::types::Learning {
    runtime
        .create_learning(LearningCreateParams {
            summary: summary.to_string(),
            scope: LearningScope {
                paths: paths.iter().map(|value| (*value).to_string()).collect(),
                tags: tags.iter().map(|value| (*value).to_string()).collect(),
                ..Default::default()
            },
            body: "body must not be injected".to_string(),
            evidence: Vec::new(),
            created_by: Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
            priority,
        })
        .expect("create learning")
}

fn task_with_context(
    runtime: &OrbitRuntime,
    context_files: Vec<String>,
    tags: Vec<String>,
) -> Task {
    std::fs::create_dir_all(runtime.paths().repo_root.join("crates/orbit-engine/src"))
        .expect("create context dir");
    runtime
        .add_task(TaskAddParams {
            title: "Learning reminder task".to_string(),
            description: "Task description.".to_string(),
            acceptance_criteria: vec!["works".to_string()],
            plan: "plan".to_string(),
            context_files,
            tags,
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add task")
}

#[test]
fn reminders_match_task_context_paths_and_tags_without_body() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    create_learning(
        &runtime,
        "Remember the engine path.",
        &["crates/orbit-engine/**"],
        &[],
        None,
    );
    create_learning(&runtime, "Remember the tag.", &[], &["workflow"], None);
    let task = task_with_context(
        &runtime,
        vec!["dir:crates/orbit-engine/src".to_string()],
        vec!["workflow".to_string()],
    );

    let reminders = runtime
        .learning_reminders_for_task(
            &json!({"task_id": task.id}),
            LearningInjectionCaps::default(),
        )
        .expect("learning reminders");

    assert_eq!(reminders.len(), 2);
    assert!(
        reminders
            .iter()
            .any(|reminder| reminder.summary == "Remember the engine path.")
    );
    assert!(
        reminders
            .iter()
            .any(|reminder| reminder.summary == "Remember the tag.")
    );
    assert!(
        !serde_json::to_string(&reminders)
            .expect("json")
            .contains("body")
    );
}

#[test]
fn reminders_apply_default_per_call_cap_after_merge() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    for idx in 0..7 {
        create_learning(
            &runtime,
            &format!("Learning {idx}"),
            &["crates/orbit-engine/**"],
            &[],
            Some(idx),
        );
    }
    let task = task_with_context(
        &runtime,
        vec!["dir:crates/orbit-engine/src".to_string()],
        Vec::new(),
    );

    let reminders = runtime
        .learning_reminders_for_task(
            &json!({"task_id": task.id}),
            LearningInjectionCaps::default(),
        )
        .expect("learning reminders");

    assert_eq!(reminders.len(), 5);
    assert_eq!(reminders[0].summary, "Learning 6");
}

#[test]
fn reminders_skip_indexed_learning_when_yaml_is_missing() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let missing = create_learning(
        &runtime,
        "Missing YAML reminder.",
        &["crates/orbit-engine/**"],
        &[],
        Some(2),
    );
    create_learning(
        &runtime,
        "Available reminder.",
        &["crates/orbit-engine/**"],
        &[],
        Some(1),
    );
    let task = task_with_context(
        &runtime,
        vec!["dir:crates/orbit-engine/src".to_string()],
        Vec::new(),
    );
    std::fs::remove_file(
        runtime
            .paths()
            .learnings_dir
            .join(&missing.id)
            .join("learning.yaml"),
    )
    .expect("remove learning yaml");

    let search_results = runtime
        .search_learnings(LearningSearchParams {
            path: Some("crates/orbit-engine/src".to_string()),
            tag: None,
            query: None,
            limit: None,
        })
        .expect("search indexed learnings");
    assert!(
        search_results
            .iter()
            .any(|result| result.learning.id == missing.id)
    );

    let reminders = runtime
        .learning_reminders_for_task(
            &json!({"task_id": task.id}),
            LearningInjectionCaps::default(),
        )
        .expect("learning reminders");

    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].summary, "Available reminder.");
    assert!(!reminders.iter().any(|reminder| reminder.id == missing.id));
}

/// ORB-10113: two workspaces bound to the same host-global database must
/// not cross-pollinate reminders. Workspace A's task never receives
/// workspace B's learning summary, while a genuine same-workspace index
/// row whose YAML body is missing is still skipped by the defensive check.
#[test]
fn reminders_never_cross_workspaces_but_skip_same_workspace_ghosts() {
    let root = tempfile::tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    std::fs::create_dir_all(&global_root).expect("global root");
    let ws_a = root.path().join("repo-a").join(".orbit");
    let ws_b = root.path().join("repo-b").join(".orbit");
    std::fs::create_dir_all(&ws_a).expect("ws a");
    std::fs::create_dir_all(&ws_b).expect("ws b");

    // Both runtimes share `global_root/orbit.db`, so they share the single
    // `learnings_index` table but have distinct registered workspace ids.
    let runtime_a = OrbitRuntime::from_roots(&global_root, &ws_a).expect("runtime a");
    let runtime_b = OrbitRuntime::from_roots(&global_root, &ws_b).expect("runtime b");

    // Workspace B owns a learning on a shared path glob. Its canonical id
    // is `L-0001`, colliding with workspace A's first record — the exact
    // duplicate-id shape that let a foreign summary leak in before scoping.
    create_learning(
        &runtime_b,
        "workspace B leak",
        &["crates/orbit-engine/**"],
        &[],
        Some(9),
    );

    // Workspace A: a genuine same-workspace ghost (index row whose YAML is
    // removed) plus a live record. The ghost must be skipped, the live one
    // kept, and B's row must never appear.
    let ghost = create_learning(
        &runtime_a,
        "workspace A ghost",
        &["crates/orbit-engine/**"],
        &[],
        Some(5),
    );
    create_learning(
        &runtime_a,
        "workspace A live",
        &["crates/orbit-engine/**"],
        &[],
        Some(1),
    );
    std::fs::remove_file(
        runtime_a
            .paths()
            .learnings_dir
            .join(&ghost.id)
            .join("learning.yaml"),
    )
    .expect("remove ghost yaml");

    let task = task_with_context(
        &runtime_a,
        vec!["dir:crates/orbit-engine/src".to_string()],
        Vec::new(),
    );
    let reminders = runtime_a
        .learning_reminders_for_task(
            &json!({"task_id": task.id}),
            LearningInjectionCaps::default(),
        )
        .expect("learning reminders");

    assert_eq!(
        reminders.len(),
        1,
        "only workspace A's live learning should remind: {reminders:?}"
    );
    assert_eq!(reminders[0].summary, "workspace A live");
    assert!(
        !reminders.iter().any(|r| r.summary == "workspace B leak"),
        "workspace B's learning must not cross into workspace A",
    );
    assert!(
        !reminders.iter().any(|r| r.id == ghost.id),
        "the same-workspace ghost (missing YAML) must be skipped",
    );
}
