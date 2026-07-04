use crate::command::task::{TaskAddParams, compute_task_add_warnings};
use orbit_common::types::{TaskStatus, TaskType};

use super::test_runtime;

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

#[test]
fn task_add_redacts_secrets_in_stored_fields() {
    // [ORB-00417] A pasted key in title/description/plan/acceptance_criteria/
    // comment must be redacted at write time so it never lands in the task
    // registry.
    let (_root, runtime) = test_runtime();

    let sk_key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcd";
    let bearer_token = "abc123def456ghi789SECRETTOKEN";
    let task = runtime
        .add_task(TaskAddParams {
            title: format!("Fix auth using {sk_key}"),
            description: format!("Header pasted: Authorization: Bearer {bearer_token}"),
            acceptance_criteria: vec![format!("no leak of {sk_key}")],
            plan: format!("call the API with {sk_key}"),
            comment: Some(format!("context: reproduce with {sk_key}")),
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("task add succeeds");

    let check = |task: &orbit_common::types::Task, label: &str| {
        assert!(
            !task.title.contains(sk_key),
            "{label}: title leaked key: {}",
            task.title
        );
        assert!(
            !task.description.contains(bearer_token),
            "{label}: description leaked bearer token: {}",
            task.description
        );
        assert!(
            !task.plan.contains(sk_key),
            "{label}: plan leaked key: {}",
            task.plan
        );
        assert!(
            !task.acceptance_criteria.iter().any(|c| c.contains(sk_key)),
            "{label}: acceptance criteria leaked key: {:?}",
            task.acceptance_criteria
        );
        assert!(
            task.title.contains("[REDACTED"),
            "{label}: title should carry a redaction placeholder: {}",
            task.title
        );
    };

    // The returned (just-created) record is redacted...
    check(&task, "returned");
    // ...and so is the persisted record read back from the store.
    let reloaded = runtime.get_task(&task.id).expect("get task");
    check(&reloaded, "reloaded");

    // The creation comment is persisted separately — it must be redacted too.
    let comments = runtime.get_task_comments(&task.id).expect("get comments");
    assert!(
        !comments.iter().any(|c| c.message.contains(sk_key)),
        "creation comment leaked key: {comments:?}"
    );
    assert!(
        comments.iter().any(|c| c.message.contains("[REDACTED")),
        "creation comment should carry a redaction placeholder: {comments:?}"
    );
}
