use super::super::open::{open_batch_pr, pr_open};
use super::test_support::*;

use crate::context::TaskReadHost;
use orbit_common::types::TaskStatus;
use serde_json::json;

#[test]
fn pr_open_rejects_missing_execution_summary_before_create() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![
            batch_task(
                "T20260430-31A",
                "First completed task",
                "## Status\nsuccess\n\n## Summary of Changes\n- First task is complete.",
            ),
            batch_task("T20260430-31B", "Second completed task", "   \n"),
        ],
        workspace.repo.clone(),
    );

    let error = open_batch_pr(
        &host,
        &pr_open_input(&workspace.repo, vec!["T20260430-31A", "T20260430-31B"]),
    )
    .expect_err("missing execution summary should reject PR creation");
    let message = error.to_string();

    assert!(message.contains("T20260430-31B"));
    assert!(message.contains("requires a meaningful persisted execution_summary"));
    assert!(message.contains("before opening the PR"));
    assert!(
        host.tool_calls()
            .iter()
            .all(|call| call.name != "github.pr.create")
    );
}

#[test]
fn pr_open_completes_no_diff_branch_without_github_pr() {
    let workspace = no_diff_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260513-16",
            "Create a user-global skill",
            "Outcome: success\n\nChanges:\n- Created files outside the repository.",
        )],
        workspace.repo.clone(),
    )
    .with_activity_implementer("codex", "codex");

    let result = pr_open(&host, &pr_open_input(&workspace.repo, vec!["T20260513-16"]))
        .expect("pr_open should complete no-diff handoff without a GitHub PR");

    assert_eq!(result["pr_created"], json!(false));
    assert_eq!(result["base"], json!("agent-main"));
    assert_eq!(result["head"], json!("orbit/test-batch"));
    assert_eq!(result["commits_behind"], json!(0));
    assert_eq!(result["commits_ahead"], json!(0));
    assert!(
        result["reason"]
            .as_str()
            .expect("no-diff reason")
            .contains("no repository commits")
    );
    assert!(host.tool_calls().is_empty());

    let task = host.get_task("T20260513-16").expect("updated task");
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.implemented_by.as_deref(), Some("codex"));
    assert!(task.external_refs.is_empty());
    assert_eq!(task.github_pr_number(), None);
}

#[test]
fn pr_open_generates_body_with_all_completed_task_summaries() {
    let workspace = pr_workspace();
    let first_summary =
        "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the first bundle task.";
    let second_summary =
        "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the second bundle task.";
    let host = PrOpenTestHost::new(
        vec![
            batch_task("T20260430-31A", "First completed task", first_summary),
            batch_task("T20260430-31B", "Second completed task", second_summary),
        ],
        workspace.repo.clone(),
    )
    .with_activity_implementer("codex", "codex");

    let result = open_batch_pr(
        &host,
        &pr_open_input(&workspace.repo, vec!["T20260430-31A", "T20260430-31B"]),
    )
    .expect("pr_open should create PR");
    assert_eq!(result["pr_created"], json!(true));
    assert_eq!(result["pr_number"], json!("42"));
    assert_eq!(
        result["pr_url"],
        json!("https://github.example/orbit/orbit/pull/42")
    );
    let body = host.pr_create_body();

    assert!(body.contains("- T20260430-31A First completed task"));
    assert!(body.contains(first_summary));
    assert!(body.contains("- T20260430-31B Second completed task"));
    assert!(body.contains(second_summary));
    assert_eq!(
        body.matches("<details><summary>Execution Summary</summary>")
            .count(),
        2
    );

    let first_task = host.get_task("T20260430-31A").expect("first task");
    let second_task = host.get_task("T20260430-31B").expect("second task");
    assert_eq!(first_task.status, TaskStatus::Review);
    assert_eq!(second_task.status, TaskStatus::Review);
    assert_eq!(first_task.implemented_by.as_deref(), Some("codex"));
    assert_eq!(second_task.implemented_by.as_deref(), Some("codex"));
    assert_eq!(first_task.github_pr_number(), Some("42"));
    assert_eq!(second_task.github_pr_number(), Some("42"));
}

#[test]
fn pr_open_preserves_non_empty_explicit_body() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260430-31A",
            "First completed task",
            "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the task.",
        )],
        workspace.repo.clone(),
    );
    let mut input = pr_open_input(&workspace.repo, vec!["T20260430-31A"]);
    input["body"] = json!("Custom reviewer handoff.");

    pr_open(&host, &input).expect("pr_open should create PR with explicit body");

    assert_eq!(host.pr_create_body(), "Custom reviewer handoff.");
}
