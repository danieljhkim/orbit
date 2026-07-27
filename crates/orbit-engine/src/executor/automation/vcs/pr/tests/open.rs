use super::super::open::pr_open;
use super::super::promote::pr_promote;
use super::test_support::*;

use super::super::body::GITHUB_PR_BODY_BYTE_LIMIT;
use crate::context::TaskReadHost;
use orbit_common::types::TaskStatus;
use serde_json::json;

#[test]
fn pr_open_rejects_missing_execution_summary_before_external_calls() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task("T20260430-31B", "Incomplete task", "   \n")],
        workspace.repo.clone(),
    );

    let error = pr_open(
        &host,
        &pr_open_input(&workspace.repo, vec!["T20260430-31B"]),
    )
    .expect_err("missing execution summary should reject PR creation");

    assert!(error.to_string().contains("T20260430-31B"));
    assert!(
        error
            .to_string()
            .contains("meaningful persisted execution_summary")
    );
    assert!(host.tool_calls().is_empty());
}

#[test]
fn pr_open_fails_loudly_on_zero_commit_branch() {
    let workspace = no_diff_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260513-16",
            "Empty branch",
            "Outcome: success\n\nChanges:\n- Nothing landed.",
        )],
        workspace.repo.clone(),
    );

    let error = pr_open(&host, &pr_open_input(&workspace.repo, vec!["T20260513-16"]))
        .expect_err("zero-commit branch must not open a PR");

    assert!(error.to_string().contains("must be ahead"));
    assert!(host.tool_calls().is_empty());
    assert_eq!(
        host.get_task("T20260513-16").expect("task").status,
        TaskStatus::InProgress
    );
    assert!(
        host.comments_for("T20260513-16")[0]
            .message
            .contains("[phase=empty-branch]")
    );
}

#[test]
fn no_diff_expected_bundle_promotes_without_pr_metadata() {
    let workspace = no_diff_pr_workspace();
    let mut task = batch_task(
        "T20260712-1",
        "Side-effect-only task",
        "Outcome: success\nChanges:\n- Filed durable external state.",
    );
    task.tags
        .push(orbit_common::types::NO_DIFF_EXPECTED_TAG.to_string());
    let host = PrOpenTestHost::new(vec![task], workspace.repo.clone())
        .with_activity_implementer("codex", "codex");

    let result = pr_promote(
        &host,
        &json!({
            "workspace_path": workspace.repo,
            "job_run_id": "batch-1",
            "completed_task_ids": ["T20260712-1"],
            "no_diff_expected": true,
        }),
    )
    .expect("promote no-diff task");

    assert_eq!(result["decision"], json!("performed"));
    let task = host.get_task("T20260712-1").expect("task");
    assert_eq!(task.status, TaskStatus::Review);
    assert!(task.external_refs.is_empty());
    assert!(host.tool_calls().is_empty());
}

#[test]
fn pr_open_creates_body_without_promoting_until_explicit_phase() {
    let workspace = pr_workspace();
    let first_summary = "Outcome: success\n\nChanges:\n- First task complete.";
    let second_summary = "Outcome: success\n\nChanges:\n- Second task complete.";
    let host = PrOpenTestHost::new(
        vec![
            batch_task("T20260430-31A", "First completed task", first_summary),
            batch_task("T20260430-31B", "Second completed task", second_summary),
        ],
        workspace.repo.clone(),
    )
    .with_activity_implementer("codex", "codex");
    let input = pr_open_input(&workspace.repo, vec!["T20260430-31A", "T20260430-31B"]);

    let result = pr_open(&host, &input).expect("create PR");
    assert_eq!(result["decision"], json!("performed"));
    assert_eq!(result["pr_number"], json!("42"));
    assert_eq!(
        host.get_task("T20260430-31A").expect("task").status,
        TaskStatus::InProgress,
        "PR creation must not hide task promotion in the same activity"
    );
    let body = host.pr_create_body();
    assert!(body.contains("First completed task"));
    assert!(body.contains(first_summary));
    assert!(body.contains("Second completed task"));
    assert!(body.contains(second_summary));
    let calls = host.tool_calls();
    let lookup = calls
        .iter()
        .find(|call| call.name == "github.pr.list")
        .expect("PR lookup must use the branch-aware list tool");
    assert_eq!(lookup.input["head"], json!("orbit/test-batch"));
    assert_eq!(lookup.input["state"], json!("open"));
    assert!(
        calls
            .iter()
            .filter(|call| call.name == "github.pr.view")
            .all(|call| call.input["pr"] != json!("orbit/test-batch")),
        "PR view only accepts numeric PR numbers or GitHub PR URLs"
    );

    let promote = json!({
        "workspace_path": workspace.repo,
        "job_run_id": "batch-1",
        "completed_task_ids": ["T20260430-31A", "T20260430-31B"],
        "pr_number": result["pr_number"],
        "pr_url": result["pr_url"],
    });
    let promoted = pr_promote(&host, &promote).expect("promote tasks");
    assert_eq!(promoted["decision"], json!("performed"));
    for task_id in ["T20260430-31A", "T20260430-31B"] {
        let task = host.get_task(task_id).expect("promoted task");
        assert_eq!(task.status, TaskStatus::Review);
        assert_eq!(task.implemented_by.as_deref(), Some("codex"));
        assert_eq!(task.github_pr_number(), Some("42"));
    }
    assert_eq!(
        pr_promote(&host, &promote).expect("retry promotion")["decision"],
        json!("reused")
    );
}

#[test]
fn pr_open_reuses_existing_branch_pr_without_create() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260716-1",
            "Reuse PR",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    )
    .with_existing_pr();

    let result =
        pr_open(&host, &pr_open_input(&workspace.repo, vec!["T20260716-1"])).expect("reuse PR");

    assert_eq!(result["decision"], json!("reused"));
    assert_eq!(result["pr_reused"], json!(true));
    assert_eq!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.create")
            .count(),
        0
    );
}

#[test]
fn pr_open_retry_after_create_then_view_failure_discovers_same_pr() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260716-2",
            "Restart PR create",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    host.queue_tool_error("github.pr.view", "temporary local PR view failure");
    let input = pr_open_input(&workspace.repo, vec!["T20260716-2"]);

    let error = pr_open(&host, &input).expect_err("post-create view failure");
    assert!(
        error
            .to_string()
            .contains("temporary local PR view failure")
    );
    assert_eq!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.create")
            .count(),
        1
    );

    let retried = pr_open(&host, &input).expect("retry discovers created PR");
    assert_eq!(retried["decision"], json!("reused"));
    assert_eq!(retried["pr_number"], json!("42"));
    assert!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.view")
            .all(|call| call.input["pr"] != json!("orbit/test-batch")),
        "retry must discover by head branch and view only a numeric PR number or URL"
    );
    assert_eq!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.create")
            .count(),
        1,
        "retry must not create a duplicate PR"
    );
}

#[test]
fn pr_open_records_phase_specific_failed_handoff_comment() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260521-2A",
            "Create failure",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    host.fail_tool("github.pr.create", "gh: HTTP 502 from api.github.com");

    let error = pr_open(&host, &pr_open_input(&workspace.repo, vec!["T20260521-2A"]))
        .expect_err("create failure propagates");
    assert!(error.to_string().contains("HTTP 502"));
    let comments = host.comments_for("T20260521-2A");
    assert_eq!(comments.len(), 1);
    assert!(comments[0].message.contains("[run=batch-1]"));
    assert!(comments[0].message.contains("[phase=github.pr.create]"));
    assert!(
        comments[0]
            .message
            .contains("Later handoff phases have not been replayed")
    );
}

#[test]
fn pr_open_preserves_non_empty_explicit_body() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "T20260430-31A",
            "Explicit body",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    let mut input = pr_open_input(&workspace.repo, vec!["T20260430-31A"]);
    input["body"] = json!("Custom reviewer handoff.");

    pr_open(&host, &input).expect("create PR with explicit body");
    assert_eq!(host.pr_create_body(), "Custom reviewer handoff.");
}

#[test]
fn pr_open_bounds_an_oversized_explicit_body_before_github_create() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "ORB-10474",
            "Bound explicit body",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    let mut input = pr_open_input(&workspace.repo, vec!["ORB-10474"]);
    input["body"] = json!("é".repeat(GITHUB_PR_BODY_BYTE_LIMIT));

    pr_open(&host, &input).expect("create PR with bounded explicit body");

    let body = host.pr_create_body();
    assert!(body.len() <= GITHUB_PR_BODY_BYTE_LIMIT);
    assert!(body.contains("**Audit note:**"));
    assert!(body.contains("ORB-10474"));
}
