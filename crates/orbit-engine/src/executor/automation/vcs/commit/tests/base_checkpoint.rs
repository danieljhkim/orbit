//! ORB-10380: the commit step reconciles the task branch against the commit
//! `worktree_setup` pinned, never against a ref name.
//!
//! `refs/remotes/origin/<base>` is shared by every worktree hanging off one
//! `.git`. A sibling run's setup fetch, a rescue fetch, or a merge moves it
//! while other runs are still in flight, so a commit step that re-resolved the
//! name failed every older run by construction. These tests pin the pinned-base
//! contract, the merge-base fallback, the ADR-0219 carve-out reachability, and
//! the rule that no failure path mutates the worktree on its way out.

use std::fs;
use std::path::Path;

use orbit_common::types::{NO_DIFF_EXPECTED_TAG, OrbitError};
use serde_json::{Value, json};

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::{git_output, git_success};

const MOVING_BASE_REF: &str = "origin/agent-main";

fn commit_all(workspace: &Path, message: &str) -> String {
    git_success(workspace, &["add", "--all", "--", "."]).expect("stage fixture change");
    git_success(workspace, &["commit", "-m", message]).expect("commit fixture change");
    git_output(workspace, &["rev-parse", "HEAD"]).expect("read fixture head")
}

/// Point the shared remote-tracking ref at `sha`, the way a fetch or a merge in
/// a sibling worktree does.
fn move_shared_base_ref(workspace: &Path, sha: &str) {
    git_success(
        workspace,
        &[
            "update-ref",
            &format!("refs/remotes/{MOVING_BASE_REF}"),
            sha,
        ],
    )
    .expect("move shared base ref");
}

fn batch_input(workspace: &Path, base_sha: &str) -> Value {
    json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": MOVING_BASE_REF,
        "base_sha": base_sha,
    })
}

#[test]
fn commit_survives_the_shared_base_ref_moving_after_worktree_setup() {
    // The regression that matters: a sibling run advances `origin/agent-main`
    // mid-run. Before ORB-10380 the commit step re-resolved that name, found the
    // new tip was not an ancestor of HEAD, and failed the whole run.
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read setup checkpoint");
    move_shared_base_ref(workspace, &base_sha);
    git_success(workspace, &["checkout", "-b", "orbit/T1"]).expect("create task branch");

    fs::write(workspace.join("task.txt"), "task work\n").unwrap();
    let task_head = commit_all(workspace, "implement-authored commit");

    // A sibling run's `worktree_setup` fetch lands a newer base.
    git_success(workspace, &["checkout", "--detach", &base_sha]).expect("detach at base");
    fs::write(workspace.join("sibling.txt"), "sibling run work\n").unwrap();
    let advanced_base = commit_all(workspace, "sibling run merged first");
    move_shared_base_ref(workspace, &advanced_base);
    git_success(workspace, &["checkout", "orbit/T1"]).expect("return to the task branch");
    assert_ne!(
        git_output(workspace, &["rev-parse", MOVING_BASE_REF]).expect("read moved base"),
        base_sha,
        "precondition: the shared base ref moved while the run was in flight"
    );

    let task = task_with_file("T1", "Pinned base task", "task.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    let result = git_commit(&host, &batch_input(workspace, &base_sha))
        .expect("a moved shared base ref must not fail the commit step");

    assert_eq!(result["decision"], "adopted_existing_commits");
    assert_eq!(result["base_sha"], base_sha);
    assert_eq!(result["commit_shas"], json!([task_head]));
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read final head"),
        task_head,
        "the pipeline must not rewrite the adopted commit"
    );
}

#[test]
fn commit_rejects_a_base_sha_input_that_is_a_ref_name() {
    // The contract is a pinned commit id. Accepting a name here would quietly
    // restore the moving-base failure.
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read checkpoint");
    move_shared_base_ref(workspace, &base_sha);

    let task = task_with_file("T1", "Pinned base task", "task.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    let error = git_commit(&host, &batch_input(workspace, MOVING_BASE_REF))
        .expect_err("a ref name is not a pinned base");

    let OrbitError::InvalidInput(message) = error else {
        panic!("expected invalid input");
    };
    assert!(
        message.contains("must be the full commit id pinned by worktree_setup"),
        "{message}"
    );
}

#[test]
fn commit_fails_with_observed_state_when_the_pinned_base_shares_no_history() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read checkpoint");

    git_success(workspace, &["checkout", "--orphan", "unrelated"])
        .expect("start unrelated history");
    git_success(workspace, &["rm", "-rf", "--cached", "."]).expect("clear orphan index");
    fs::write(workspace.join("unrelated.txt"), "unrelated root\n").unwrap();
    let unrelated_head = commit_all(workspace, "unrelated root commit");

    let task = task_with_file("T1", "Unrelated history", "unrelated.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    let error = git_commit(&host, &batch_input(workspace, &base_sha))
        .expect_err("unrelated histories remain a hard failure");

    let message = error.to_string();
    assert!(message.contains("shares no history"), "{message}");
    assert!(
        message.contains(&base_sha),
        "names the pinned base: {message}"
    );
    assert!(message.contains(&unrelated_head), "names HEAD: {message}");
    assert!(
        !message.contains("nothing to commit"),
        "the ancestry failure must not reuse the empty-stage wording: {message}"
    );
}

#[test]
fn unrelated_history_failure_leaves_the_worktree_exactly_as_found() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read checkpoint");

    git_success(workspace, &["checkout", "--orphan", "unrelated"])
        .expect("start unrelated history");
    git_success(workspace, &["rm", "-rf", "--cached", "."]).expect("clear orphan index");
    fs::write(workspace.join("unrelated.txt"), "unrelated root\n").unwrap();
    let head_before = commit_all(workspace, "unrelated root commit");

    // Leave the checkout dirty in both directions: one staged change, one
    // untracked file. A failure path must touch neither.
    fs::write(workspace.join("unrelated.txt"), "edited in place\n").unwrap();
    git_success(workspace, &["add", "unrelated.txt"]).expect("stage an edit");
    fs::write(workspace.join("scratch.txt"), "untracked scratch\n").unwrap();
    let index_before = git_stdout_bytes(
        workspace,
        &["diff", "--cached", "--binary", "HEAD", "--"],
        "snapshot index before",
    );
    let status_before = git_output(
        workspace,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .expect("snapshot status before");

    let task = task_with_file("T1", "Unrelated history", "unrelated.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    git_commit(&host, &batch_input(workspace, &base_sha)).expect_err("the run fails");

    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read head after"),
        head_before,
        "no commit may be created on a failure path"
    );
    assert_eq!(
        git_stdout_bytes(
            workspace,
            &["diff", "--cached", "--binary", "HEAD", "--"],
            "snapshot index after",
        ),
        index_before,
        "the index must be left as found"
    );
    assert_eq!(
        git_output(
            workspace,
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .expect("snapshot status after"),
        status_before,
        "the worktree must be left as found"
    );
}

#[test]
fn empty_stage_failure_leaves_the_index_as_found() {
    // ORB-10380: the old empty-diff branch ran `git reset HEAD` on its way out.
    // A failure path never mutates the checkout it is reporting on.
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read checkpoint");
    let index_before = git_stdout_bytes(
        workspace,
        &["diff", "--cached", "--binary", "HEAD", "--"],
        "snapshot index before",
    );

    let task = task_with_file("T1", "Empty task", "src/missing.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    git_commit(&host, &batch_input(workspace, &base_sha)).expect_err("empty stage errors");

    assert_eq!(
        git_stdout_bytes(
            workspace,
            &["diff", "--cached", "--binary", "HEAD", "--"],
            "snapshot index after",
        ),
        index_before
    );
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read head after"),
        base_sha
    );
}

#[test]
fn no_diff_expected_task_skips_the_phase_even_when_its_base_is_unreachable() {
    // ADR-0219's carve-out used to live only on the empty-stage branch, so a
    // side-effect-only task whose history could not be reconciled hard-failed
    // instead of skipping. The carve-out is now evaluated on both branches.
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read checkpoint");

    git_success(workspace, &["checkout", "--orphan", "unrelated"])
        .expect("start unrelated history");
    git_success(workspace, &["rm", "-rf", "--cached", "."]).expect("clear orphan index");
    fs::write(workspace.join("unrelated.txt"), "unrelated root\n").unwrap();
    let head_before = commit_all(workspace, "unrelated root commit");

    let mut task = task_with_file("T1", "QA validation", "src/missing.txt", "sonnet");
    task.tags.push(NO_DIFF_EXPECTED_TAG.to_string());
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());

    let result = git_commit(&host, &batch_input(workspace, &base_sha))
        .expect("a side-effect-only task skips the phase");

    assert_eq!(result["skipped_no_diff_expected"], json!(true));
    assert_eq!(result["decision"], "skipped_no_diff_expected");
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read head after"),
        head_before,
        "the skip creates no commit"
    );
}
