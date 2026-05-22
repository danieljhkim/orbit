use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, compute_task_add_warnings};
use orbit_common::types::{TaskStatus, TaskType};
use tempfile::tempdir;

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime)
}

#[test]
fn task_add_enters_proposed_and_requires_approval_before_backlog() {
    let (_root, runtime) = test_runtime();

    let task = runtime
        .add_task(TaskAddParams {
            title: "Create orbit hello".to_string(),
            description: "Add a small hello file.".to_string(),
            acceptance_criteria: vec!["orbit-hello.txt exists.".to_string()],
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("human task add succeeds");

    assert_eq!(task.status, TaskStatus::Proposed);

    let approved = runtime
        .approve_task(&task.id, Some("LGTM".to_string()), None)
        .expect("proposed task can be approved into backlog");
    assert_eq!(approved.status, TaskStatus::Backlog);

    let started = runtime
        .start_task(&task.id, Some("start approved task".to_string()), None)
        .expect("backlog task starts directly");
    assert_eq!(started.status, TaskStatus::InProgress);
}

#[test]
fn task_add_rejects_legacy_friction_status() {
    let (_root, runtime) = test_runtime();

    let err = runtime
        .add_task(TaskAddParams {
            title: "Friction type".to_string(),
            description: "Legacy friction path.".to_string(),
            task_type: Some(TaskType::Chore),
            status: Some(TaskStatus::Friction),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect_err("status friction should fail");
    assert!(err.to_string().contains("use orbit.friction.add"), "{err}");

    let task = runtime
        .add_task(TaskAddParams {
            title: "Chore type".to_string(),
            description: "Modern task type path.".to_string(),
            task_type: Some(TaskType::Chore),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("chore type still succeeds");
    assert_eq!(task.task_type, TaskType::Chore);

    let err = runtime
        .add_task(TaskAddParams {
            title: "Friction status".to_string(),
            description: "Legacy friction path.".to_string(),
            status: Some(TaskStatus::Friction),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect_err("status friction should fail");
    assert!(err.to_string().contains("use orbit.friction.add"), "{err}");
}

// --- ORB-00251: context_files omission / over-inclusion warning helper tests ---

#[test]
fn add_task_warnings_omission_for_non_chore_empty_context() {
    // (a) non-chore + empty -> omission present, over absent
    let w = compute_task_add_warnings(&[], TaskType::Feature);
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("without context_files"));
    assert!(!w[0].contains("reference material"));
}

#[test]
fn add_task_warnings_none_for_non_chore_with_only_targets() {
    // (b)
    let w = compute_task_add_warnings(
        &["file:src/main.rs".to_string(), "dir:crates/foo".to_string()],
        TaskType::Bug,
    );
    assert!(w.is_empty());
}

#[test]
fn add_task_warnings_none_for_chore_empty() {
    // (c) chore + empty -> no warnings
    let w = compute_task_add_warnings(&[], TaskType::Chore);
    assert!(w.is_empty());
}

#[test]
fn add_task_warnings_over_inclusion_for_design_patterns() {
    // (d) non-chore + design-patterns entry -> over present, omission absent
    let w = compute_task_add_warnings(
        &["file:docs/design-patterns/test_layout.md".to_string()],
        TaskType::Refactor,
    );
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("reference material"));
    assert!(w[0].contains("docs/design-patterns/test_layout.md"));
    assert!(!w[0].contains("without context_files"));
}

#[test]
fn add_task_warnings_no_over_inclusion_for_feature_design_doc() {
    // (e) feature design docs are excluded from over-inclusion
    let w = compute_task_add_warnings(
        &["file:docs/design/some-feature/2_design.md".to_string()],
        TaskType::Feature,
    );
    assert!(w.is_empty());
}

#[test]
fn add_task_warnings_mixed_valid_and_claude_over_only() {
    // (f) mix valid + CLAUDE.md -> over naming only the bad one; no omission
    let w = compute_task_add_warnings(
        &["file:src/foo.rs".to_string(), "file:CLAUDE.md".to_string()],
        TaskType::Feature,
    );
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("reference material"));
    assert!(w[0].contains("CLAUDE.md"));
    assert!(!w[0].contains("without context_files"));
    assert!(!w[0].contains("src/foo.rs"));
}
