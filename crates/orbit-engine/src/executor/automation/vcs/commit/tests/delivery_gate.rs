//! ORB-10313 regression: `commit_batch_changes` must fail closed on the durable
//! execution outcome before it resolves the checkout, stages files, mutates the
//! index, or creates a commit. A `failed`, missing, or unknown outcome — like an
//! empty/placeholder summary — leaves HEAD, the index, and the worktree exactly
//! as they were.

use std::fs;
use std::path::Path;

use orbit_common::types::Task;
use serde_json::{Value, json};

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::git_output;

const GATED_TASK_ID: &str = "ORB-10313-GATE";

fn task_with_summary(summary: &str) -> Task {
    let mut task = task_with_file(
        GATED_TASK_ID,
        "Deliver gated work",
        "src/change.txt",
        "codex",
    );
    task.execution_summary = summary.to_string();
    task
}

fn batch_input(workspace: &Path) -> Value {
    json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    })
}

/// Stage a fresh repo with an uncommitted change on disk, run the batch commit
/// with the given summary, and assert the delivery gate rejected it without
/// touching HEAD, the index, or the worktree.
fn assert_delivery_blocked(summary: &str, expected_fragment: &str) {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/change.txt"), "would-be delivery\n").unwrap();

    let head_before = git_output(workspace, &["rev-parse", "HEAD"]).expect("HEAD before");
    let status_before = git_output(
        workspace,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .expect("worktree status before");
    assert_eq!(
        status_before.trim(),
        "?? src/change.txt",
        "precondition: the change is present and unstaged"
    );

    let host = CommitTestHost::new(vec![task_with_summary(summary)], workspace.to_path_buf());
    let error = git_commit(&host, &batch_input(workspace))
        .expect_err("non-success outcome must block delivery");
    let message = error.to_string();
    assert!(
        message.contains(GATED_TASK_ID),
        "error names the task: {message}"
    );
    assert!(
        message.contains(expected_fragment),
        "error names the rejected value ({expected_fragment}): {message}"
    );

    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("HEAD after"),
        head_before,
        "delivery gate must not create a commit"
    );
    assert_eq!(
        git_output(workspace, &["rev-list", "--count", "HEAD"])
            .expect("commit count")
            .trim(),
        "1",
        "only the initial commit remains"
    );
    assert!(
        git_output(workspace, &["diff", "--cached", "--name-only"])
            .expect("staged files after")
            .trim()
            .is_empty(),
        "delivery gate must run before any index staging"
    );
    assert_eq!(
        git_output(
            workspace,
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .expect("worktree status after")
        .trim(),
        "?? src/change.txt",
        "worktree change is left exactly as the implement step produced it"
    );
}

#[test]
fn commit_batch_blocks_failed_outcome_before_any_git_mutation() {
    assert_delivery_blocked(
        "Outcome: failed\n\nChanges:\n- Critical scope unimplemented.",
        "failed",
    );
}

#[test]
fn commit_batch_blocks_missing_outcome_before_any_git_mutation() {
    assert_delivery_blocked(
        "Changes:\n- Did work but never stated an outcome.",
        "Changes:",
    );
}

#[test]
fn commit_batch_blocks_unknown_outcome_before_any_git_mutation() {
    assert_delivery_blocked(
        "Outcome: partial\n\nChanges:\n- Some of the work landed.",
        "partial",
    );
}

#[test]
fn commit_batch_blocks_empty_summary_before_any_git_mutation() {
    // Empty/placeholder rejection remains intact under the stricter predicate.
    assert_delivery_blocked("   \n", "meaningful persisted execution_summary");
}

#[test]
fn commit_batch_succeeds_on_success_outcome() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/change.txt"), "delivered\n").unwrap();

    let host = CommitTestHost::new(
        vec![task_with_summary(
            "Outcome: success\n\nChanges:\n- Landed the scoped work.",
        )],
        workspace.to_path_buf(),
    );
    let result = git_commit(&host, &batch_input(workspace)).expect("success outcome delivers");
    assert_eq!(result["committed"], json!(true));
    assert_eq!(result["task_id"], json!(GATED_TASK_ID));
    assert_eq!(
        git_output(workspace, &["rev-list", "--count", "HEAD"])
            .expect("commit count")
            .trim(),
        "2",
        "the success outcome produces exactly one delivery commit"
    );
}
