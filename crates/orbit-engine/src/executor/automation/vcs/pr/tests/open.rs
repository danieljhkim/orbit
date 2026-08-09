use super::super::open::pr_open;
use super::super::promote::pr_promote;
use super::test_support::*;

use super::super::body::GITHUB_PR_BODY_BYTE_LIMIT;
use crate::context::RuntimeHost;
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
fn ordinary_stacked_delivery_against_a_live_base_still_ships() {
    // ORB-10644 guard rail: the gate must cost a live stacked base nothing.
    let workspace = stacked_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "ORB-10644",
            "Child of a live stacked base",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    )
    .with_activity_implementer("claude", "claude");
    let input = stacked_pr_open_input(&workspace.repo, vec!["ORB-10644"]);

    let opened = pr_open(&host, &input).expect("live stacked base must still deliver");
    assert_eq!(opened["decision"], json!("performed"));
    assert_eq!(opened["base"], json!(STACKED_BASE_BRANCH));

    let promoted = pr_promote(
        &host,
        &promote_input_from(&input, &opened["pr_number"], &opened["pr_url"]),
    )
    .expect("live stacked base must still promote");
    assert_eq!(promoted["decision"], json!("performed"));
    assert_eq!(
        host.get_task("ORB-10644").expect("task").status,
        TaskStatus::Review
    );
}

#[test]
fn resume_refuses_to_open_or_promote_once_the_stacked_base_has_landed() {
    // The failure ORB-10644 replaces surfaced on resume: the first attempt runs
    // against a live base, the intermediate branch lands and is left behind at
    // its pre-merge tip, and every later step still reports success against a
    // PR nobody merges again.
    let workspace = stacked_pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "ORB-10644",
            "Child of a landed stacked base",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    let input = stacked_pr_open_input(&workspace.repo, vec!["ORB-10644"]);
    pr_open(&host, &input).expect("first attempt runs against a live base");

    land_stacked_base_by_squash(&workspace.repo);

    let error = pr_open(&host, &input).expect_err("resume must refuse the landed base");
    let message = error.to_string();
    assert!(
        message.contains(STACKED_BASE_BRANCH) && message.contains("agent-main"),
        "the diagnostic must name the stale base and the landing branch: {message}"
    );
    assert!(
        message.contains("[ORB-10643]"),
        "the diagnostic must name the marker that already landed: {message}"
    );
    assert!(
        message.contains("Recovery: re-dispatch this run with base 'agent-main'"),
        "the diagnostic must name a recovery path: {message}"
    );
    assert_eq!(
        host.tool_calls()
            .iter()
            .filter(|call| call.name == "github.pr.create")
            .count(),
        1,
        "the resumed attempt must not reach GitHub at all"
    );

    let promote_error = pr_promote(
        &host,
        &promote_input_from(&input, &json!("42"), &json!(null)),
    )
    .expect_err("promotion must refuse the same base");
    assert!(promote_error.to_string().contains(STACKED_BASE_BRANCH));
    assert_eq!(
        host.get_task("ORB-10644").expect("task").status,
        TaskStatus::InProgress,
        "a refused delivery must not promote the task"
    );
    assert!(
        host.comments_for("ORB-10644")
            .iter()
            .any(|comment| comment.message.contains("[phase=obsolete-base]")),
        "the refusal must leave a phase-specific handoff comment"
    );
}

#[test]
fn delivery_success_is_counted_only_when_the_commit_reaches_the_landing_branch() {
    // Obsolete base: refused, and merging into it would indeed have left the
    // commit off the landing branch.
    let stacked = stacked_pr_workspace();
    let stacked_host = PrOpenTestHost::new(
        vec![batch_task(
            "ORB-10644",
            "Child of a landed stacked base",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        stacked.repo.clone(),
    );
    land_stacked_base_by_squash(&stacked.repo);
    let head_sha = git(&stacked.repo, &["rev-parse", "orbit/test-batch"]);

    pr_open(
        &stacked_host,
        &stacked_pr_open_input(&stacked.repo, vec!["ORB-10644"]),
    )
    .expect_err("an obsolete base must not open a PR");
    git(&stacked.repo, &["checkout", STACKED_BASE_BRANCH]);
    git(
        &stacked.repo,
        &["merge", "--no-ff", "-m", "merge child", "orbit/test-batch"],
    );
    assert!(
        !is_ancestor(&stacked.repo, &head_sha, "agent-main"),
        "merging into the landed base leaves the commit off the landing branch — \
         exactly the success this gate refuses to count"
    );
    assert_eq!(
        stacked_host.get_task("ORB-10644").expect("task").status,
        TaskStatus::InProgress
    );

    // Same-base delivery: the base *is* the landing branch, so the merged
    // commit reaches it and the success is real.
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![batch_task(
            "ORB-10644",
            "Same-base delivery",
            "Outcome: success\nChanges:\n- Ready.",
        )],
        workspace.repo.clone(),
    );
    let mut input = pr_open_input(&workspace.repo, vec!["ORB-10644"]);
    input["landing_branch"] = json!("agent-main");
    let delivered_sha = git(&workspace.repo, &["rev-parse", "orbit/test-batch"]);

    let opened = pr_open(&host, &input).expect("same-base delivery must succeed");
    let promoted = pr_promote(
        &host,
        &promote_input_from(&input, &opened["pr_number"], &opened["pr_url"]),
    )
    .expect("same-base promotion must succeed");
    assert_eq!(promoted["decision"], json!("performed"));
    assert_eq!(
        host.get_task("ORB-10644").expect("task").status,
        TaskStatus::Review
    );

    git(&workspace.repo, &["checkout", "agent-main"]);
    git(
        &workspace.repo,
        &["merge", "--no-ff", "-m", "merge child", "orbit/test-batch"],
    );
    assert!(
        is_ancestor(&workspace.repo, &delivered_sha, "agent-main"),
        "a counted delivery must be reachable from the landing branch"
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
