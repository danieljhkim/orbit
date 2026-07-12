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
fn pr_open_fails_loudly_on_empty_worktree_diff() {
    // Regression (ORB-10134): when the implement step writes nothing into the
    // worktree, `pr_open` must fail at the commit step — never push an empty
    // branch, open a PR, or promote the task to review.
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

    let error = pr_open(&host, &pr_open_input(&workspace.repo, vec!["T20260513-16"]))
        .expect_err("empty worktree diff must fail loudly, not promote to review");
    let message = error.to_string();
    assert!(
        message.contains("no staged changes to commit"),
        "expected empty-diff commit error, got: {message}"
    );

    // No branch push and no PR creation ever run.
    assert!(
        host.tool_calls().is_empty(),
        "no git.push or github.pr.create should run on an empty diff, got: {:?}",
        host.tool_calls()
    );

    // The task is NOT promoted to review — it stays in-progress so the run can
    // terminalize and block it for inspection.
    let task = host.get_task("T20260513-16").expect("task still exists");
    assert_eq!(task.status, TaskStatus::InProgress);
    assert!(task.external_refs.is_empty());
    assert_eq!(task.github_pr_number(), None);
}

#[test]
fn open_batch_pr_rejects_zero_commits_ahead_branch() {
    // Even if a commit somehow exists but the head branch carries 0 commits
    // ahead of base, `open_batch_pr` must fail rather than pushing the empty
    // branch and promoting tasks to review (ORB-10134).
    let workspace = no_diff_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260513-16",
            "Create a user-global skill",
            "Outcome: success\n\nChanges:\n- Nothing landed on the branch.",
        )],
        workspace.repo.clone(),
    );

    let error = open_batch_pr(&host, &pr_open_input(&workspace.repo, vec!["T20260513-16"]))
        .expect_err("a branch with 0 commits ahead of base must fail, not promote to review");
    let message = error.to_string();
    assert!(
        message.contains("0 commits ahead"),
        "expected empty-branch error, got: {message}"
    );

    assert!(
        host.tool_calls()
            .iter()
            .all(|call| call.name != "github.pr.create"),
        "github.pr.create must not run for an empty branch"
    );

    // The task stays in-progress and a failed-handoff comment is recorded.
    let task = host.get_task("T20260513-16").expect("task still exists");
    assert_eq!(task.status, TaskStatus::InProgress);
    assert!(task.external_refs.is_empty());
    assert_eq!(task.github_pr_number(), None);

    let comments = host.comments_for("T20260513-16");
    assert_eq!(comments.len(), 1, "exactly one failed-handoff comment");
    let body = &comments[0].message;
    assert!(
        body.contains("[op=empty-branch]"),
        "handoff op should be empty-branch: {body}"
    );
    assert!(body.contains("0 commits ahead"), "handoff body: {body}");
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
fn pr_open_records_failed_handoff_comment_when_rebase_fails() {
    let workspace = rebase_conflict_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260521-1A",
            "Rebase conflict task",
            "## Status\nsuccess\n\n## Summary of Changes\n- Implemented.",
        )],
        workspace.repo.clone(),
    );

    let error = open_batch_pr(&host, &pr_open_input(&workspace.repo, vec!["T20260521-1A"]))
        .expect_err("rebase failure should propagate");
    assert!(
        error.to_string().contains("behind base"),
        "expected behind-base error, got: {error}"
    );
    assert!(
        host.tool_calls()
            .iter()
            .all(|call| call.name != "github.pr.create"),
        "PR create should not be attempted after rebase failure"
    );

    let task = host.get_task("T20260521-1A").expect("task still exists");
    assert_eq!(
        task.status,
        TaskStatus::InProgress,
        "task should remain in-progress after failed handoff"
    );

    let comments = host.comments_for("T20260521-1A");
    assert_eq!(comments.len(), 1, "exactly one failed-handoff comment");
    let comment = &comments[0];
    assert_eq!(comment.by, "system");
    let body = &comment.message;
    assert!(body.contains("pr_open handoff failed"));
    assert!(body.contains("[run=batch-1]"));
    assert!(body.contains("[op=rebase]"));
    assert!(body.contains("Head branch: orbit/test-batch"));
    assert!(body.contains("Base branch: agent-main"));
    assert!(body.contains(&workspace.repo.display().to_string()));
    assert!(body.contains("Recovery:"));
    assert!(body.contains("git rebase origin/agent-main"));
    assert!(body.contains("git push --force-with-lease origin orbit/test-batch"));

    // Idempotency: retrying the same run + branch must not append duplicate noise.
    let _ = open_batch_pr(&host, &pr_open_input(&workspace.repo, vec!["T20260521-1A"]))
        .expect_err("rebase still fails on retry");
    assert_eq!(
        host.comments_for("T20260521-1A").len(),
        1,
        "repeated failures from the same run/op must not spam duplicate comments"
    );
}

#[test]
fn pr_open_records_failed_handoff_comment_when_pr_create_fails() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![
            batch_task(
                "T20260521-2A",
                "First completed task",
                "## Status\nsuccess\n\n## Summary of Changes\n- First.",
            ),
            batch_task(
                "T20260521-2B",
                "Second completed task",
                "## Status\nsuccess\n\n## Summary of Changes\n- Second.",
            ),
        ],
        workspace.repo.clone(),
    );
    host.fail_tool("github.pr.create", "gh: HTTP 502 from api.github.com");

    let error = open_batch_pr(
        &host,
        &pr_open_input(&workspace.repo, vec!["T20260521-2A", "T20260521-2B"]),
    )
    .expect_err("github.pr.create failure should propagate");
    assert!(error.to_string().contains("HTTP 502"));

    for task_id in ["T20260521-2A", "T20260521-2B"] {
        let task = host.get_task(task_id).expect("task still exists");
        assert_eq!(
            task.status,
            TaskStatus::InProgress,
            "{task_id} should remain in-progress after pr_create failure"
        );

        let comments = host.comments_for(task_id);
        assert_eq!(
            comments.len(),
            1,
            "{task_id}: exactly one failed-handoff comment"
        );
        let body = &comments[0].message;
        assert!(body.contains("[run=batch-1]"));
        assert!(body.contains("[op=github.pr.create]"));
        assert!(body.contains("Head branch: orbit/test-batch"));
        assert!(body.contains("Base branch: agent-main"));
        assert!(body.contains("Base ref:"));
        assert!(body.contains("gh: HTTP 502 from api.github.com"));
        assert!(body.contains("gh pr create --base agent-main --head orbit/test-batch"));
    }

    // A distinct later failure (different op) still appends a new note.
    host.fail_tool("git.push", "remote rejected force-with-lease");
    let _ = open_batch_pr(
        &host,
        &pr_open_input(&workspace.repo, vec!["T20260521-2A", "T20260521-2B"]),
    )
    .expect_err("git.push failure on retry should propagate");
    let comments = host.comments_for("T20260521-2A");
    assert_eq!(
        comments.len(),
        2,
        "distinct failing op should append a new note"
    );
    assert!(comments[1].message.contains("[op=push]"));
    assert!(
        comments[1]
            .message
            .contains("remote rejected force-with-lease")
    );
}

#[test]
fn pr_open_preserves_non_empty_explicit_body() {
    // Exercises the full pr_open path (commit uncommitted worktree changes,
    // then open the PR) against a worktree whose implement-step changes are not
    // yet committed — the real PR pipeline shape.
    let workspace = pr_workspace_uncommitted();
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
