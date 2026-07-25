use std::fs;

use orbit_common::types::{OrbitError, TaskType};
use serde_json::json;

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::{git_output, git_success};

#[test]
fn git_commit_uses_scoped_identity_without_mutating_local_human_config() {
    let cases = [
        ("claude-opus-4-7", "claude <claude@orbit.local>"),
        ("gemini-3.1-pro", "gemini <gemini@orbit.local>"),
        ("gpt-5.5", "codex <codex@orbit.local>"),
        ("grok-4", "grok <grok@orbit.local>"),
        ("grok-build", "grok <grok@orbit.local>"),
        ("mystery-model", "mystery-model <mystery-model@orbit.local>"),
    ];

    for (implemented_by, expected_author) in cases {
        let temp = initialized_git_repo();
        let workspace = temp.path();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("src/task.txt"),
            format!("implemented by {implemented_by}\n"),
        )
        .unwrap();

        let task = task_with_file("T1", "Implement one task", "src/task.txt", implemented_by);
        let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
        let input = json!({
            "scope": "per_task",
            "job_run_id": "batch-1",
            "workspace_path": workspace.to_string_lossy().to_string(),
            "completed_task_ids": ["T1"],
        });

        let user_name_before = git_output(workspace, &["config", "--get", "user.name"])
            .expect("read git user.name before");
        let user_email_before = git_output(workspace, &["config", "--get", "user.email"])
            .expect("read git user.email before");
        let local_user_name_before = git_stdout_bytes(
            workspace,
            &["config", "--local", "--get", "user.name"],
            "read local git user.name before",
        );
        let local_user_email_before = git_stdout_bytes(
            workspace,
            &["config", "--local", "--get", "user.email"],
            "read local git user.email before",
        );

        git_commit(&host, &input).expect("git_commit succeeds");

        let actual_author =
            git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read git author");
        let actual_committer = git_output(workspace, &["log", "-1", "--format=%cn <%ce>"])
            .expect("read git committer");
        assert_eq!(actual_author, expected_author);
        assert_eq!(actual_committer, expected_author);
        assert_eq!(
            git_output(workspace, &["config", "--get", "user.name"])
                .expect("read git user.name after"),
            user_name_before
        );
        assert_eq!(
            git_output(workspace, &["config", "--get", "user.email"])
                .expect("read git user.email after"),
            user_email_before
        );
        assert_eq!(
            git_stdout_bytes(
                workspace,
                &["config", "--local", "--get", "user.name"],
                "read local git user.name after",
            ),
            local_user_name_before
        );
        assert_eq!(
            git_stdout_bytes(
                workspace,
                &["config", "--local", "--get", "user.email"],
                "read local git user.email after",
            ),
            local_user_email_before
        );
    }
}

#[test]
fn git_commit_succeeds_without_creating_local_user_config() {
    let temp = initialized_git_repo_without_local_user_config();
    let workspace = temp.path();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/task.txt"), "codex work\n").unwrap();

    let task = task_with_file("T1", "Implement one task", "src/task.txt", "gpt-5.5");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "per_task",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "completed_task_ids": ["T1"],
    });

    let local_user_config_before = local_user_config_snapshot(workspace);

    git_commit(&host, &input).expect("git_commit succeeds without local user config");

    let actual_author =
        git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read author");
    let actual_committer =
        git_output(workspace, &["log", "-1", "--format=%cn <%ce>"]).expect("read committer");
    assert_eq!(actual_author, "codex <codex@orbit.local>");
    assert_eq!(actual_committer, "codex <codex@orbit.local>");
    assert_eq!(
        local_user_config_snapshot(workspace),
        local_user_config_before
    );
}

#[test]
fn git_commit_batch_uses_templated_single_task_message() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/bug.txt"), "bug fix\n").unwrap();

    let title = "a".repeat(145);
    let mut task = task_with_file("ORB-00107", &title, "src/bug.txt", "claude");
    task.task_type = TaskType::Bug;
    task.planned_by = Some("codex".to_string());
    task.implemented_by = Some("claude".to_string());
    task.external_refs = vec![external_ref("eng", "1234")];
    task.execution_summary =
        "Outcome: success\n\n## Summary\n- Fixed deterministic batch commit messages.\n\n## Validation\n- cargo test"
            .to_string();
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    let local_user_config_before = local_user_config_snapshot(workspace);

    git_commit(&host, &input).expect("git_commit succeeds");

    let actual_author =
        git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read git author");
    let actual_committer =
        git_output(workspace, &["log", "-1", "--format=%cn <%ce>"]).expect("read git committer");
    let body = git_output(workspace, &["log", "-1", "--format=%B"]).expect("read git body");
    let expected_body = format!(
        "fix: {}… [ORB-00107] [ENG-1234]\n\n{}\n\nFixed deterministic batch commit messages.\n\nPlanned-By: codex\nImplemented-By: claude",
        "a".repeat(66),
        title
    );
    assert_eq!(actual_author, "claude <claude@orbit.local>");
    assert_eq!(actual_committer, "claude <claude@orbit.local>");
    assert_eq!(body, expected_body);
    assert_eq!(
        local_user_config_snapshot(workspace),
        local_user_config_before
    );
}

#[test]
fn git_commit_batch_errors_on_empty_stage() {
    // Regression (ORB-10134): a clean worktree (implement step wrote nothing)
    // must make `commit_batch_changes` error, not return a silent `Ok`.
    let temp = initialized_git_repo();
    let workspace = temp.path();

    let tasks = vec![task_with_file(
        "T1",
        "Empty task",
        "src/missing.txt",
        "claude-opus-4-7",
    )];
    let host = CommitTestHost::new(tasks, workspace.to_path_buf());
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("empty stage must error");
    let OrbitError::Execution(message) = error else {
        panic!("expected execution error, got: {error}");
    };
    assert_eq!(
        message,
        format!(
            "commit_batch_changes: no staged changes to commit for task 'T1' in worktree '{}'; \
             the implement step produced an empty diff. Changes may have been written outside \
             the assigned worktree, or attribution may be unknown; Orbit did not inspect, stage, \
             reset, or reconcile any other checkout",
            workspace.display()
        ),
        "the existing empty-diff diagnostic must remain unchanged"
    );

    // The index is left clean (the staging reset ran before erroring), so no
    // commit was created beyond the repo's initial commit.
    let log = git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count commits");
    assert_eq!(log.trim(), "1", "no commit should have been created");
}

#[test]
fn git_commit_batch_adopts_implement_authored_commits_without_rewriting_them() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");

    fs::write(workspace.join("README.md"), "reverted\n").unwrap();
    git_success(workspace, &["add", "README.md"]).expect("stage revert");
    git_success(
        workspace,
        &[
            "-c",
            "user.name=Implement Agent",
            "-c",
            "user.email=implement@example.test",
            "commit",
            "-m",
            "revert prior change",
        ],
    )
    .expect("author revert commit");
    fs::write(workspace.join("README.md"), "replacement\n").unwrap();
    git_success(workspace, &["add", "README.md"]).expect("stage replacement");
    git_success(
        workspace,
        &[
            "-c",
            "user.name=Implement Agent",
            "-c",
            "user.email=implement@example.test",
            "commit",
            "-m",
            "cherry-pick replacement",
        ],
    )
    .expect("author replacement commit");

    let head_before = git_output(workspace, &["rev-parse", "HEAD"]).expect("read authored head");
    let authored_shas = git_output(
        workspace,
        &["rev-list", "--reverse", &format!("{base_sha}..HEAD")],
    )
    .expect("read authored commits")
    .lines()
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    let task = task_with_file("T1", "History surgery", "README.md", "codex");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": base_sha,
    });

    let result = git_commit(&host, &input).expect("commit-only batch succeeds");

    assert_eq!(result["decision"], "adopted_existing_commits");
    assert_eq!(result["committed"], false);
    assert_eq!(result["adopted_commits"], true);
    assert_eq!(result["task_id"], "T1");
    assert_eq!(result["job_run_id"], "batch-1");
    assert_eq!(result["commit_shas"], json!(authored_shas));
    assert_eq!(result["commit_sha"], head_before);
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read final head"),
        head_before,
        "the pipeline must not create or amend a commit"
    );
    assert_eq!(
        git_output(workspace, &["log", "-2", "--format=%an <%ae>"]).expect("read retained authors"),
        "Implement Agent <implement@example.test>\nImplement Agent <implement@example.test>"
    );
}

#[test]
fn git_commit_batch_commits_dirty_residue_above_implement_authored_commits() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");

    fs::write(workspace.join("README.md"), "implement commit\n").unwrap();
    git_success(workspace, &["add", "README.md"]).expect("stage implement commit");
    git_success(
        workspace,
        &[
            "-c",
            "user.name=Implement Agent",
            "-c",
            "user.email=implement@example.test",
            "commit",
            "-m",
            "implement authored",
        ],
    )
    .expect("author implement commit");
    let authored_sha =
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read implement commit");
    fs::write(workspace.join("residue.txt"), "dirty residue\n").unwrap();

    let task = task_with_file("T1", "Mixed history", "residue.txt", "codex");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": base_sha,
    });

    let result = git_commit(&host, &input).expect("mixed batch succeeds");

    assert_eq!(result["decision"], "performed");
    assert_eq!(result["committed"], true);
    assert_eq!(result["adopted_commits"], true);
    assert_eq!(result["commit_shas"][0], authored_sha);
    assert_eq!(result["commit_shas"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        git_output(workspace, &["log", "-1", "--format=%P"]).expect("read residue parent"),
        authored_sha
    );
    assert_eq!(
        git_output(
            workspace,
            &["show", "-s", "--format=%an <%ae>", &authored_sha]
        )
        .expect("read retained implement author"),
        "Implement Agent <implement@example.test>"
    );
    assert_eq!(
        git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read residue author"),
        "codex <codex@orbit.local>"
    );
}

#[test]
fn git_commit_batch_does_not_adopt_commits_reachable_only_from_another_branch() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");

    git_success(workspace, &["checkout", "-b", "other-worktree-branch"])
        .expect("create other branch");
    fs::write(workspace.join("other.txt"), "other branch work\n").unwrap();
    git_success(workspace, &["add", "other.txt"]).expect("stage other branch work");
    git_success(workspace, &["commit", "-m", "other branch commit"])
        .expect("commit other branch work");
    git_success(
        workspace,
        &["checkout", "-b", "assigned-task-branch", &base_sha],
    )
    .expect("restore assigned branch at checkpoint");

    let task = task_with_file("T1", "Assigned task", "src/missing.txt", "codex");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("other branch commit is not adopted");
    let message = error.to_string();
    assert!(message.contains("no staged changes to commit"), "{message}");
    assert!(
        message.contains("outside the assigned worktree"),
        "{message}"
    );
    assert_eq!(
        git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count assigned commits"),
        "1"
    );
}

#[test]
fn git_commit_batch_rejects_head_that_is_not_a_descendant_of_the_base_checkpoint() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let root_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read root");

    fs::write(workspace.join("base.txt"), "recorded base\n").unwrap();
    git_success(workspace, &["add", "base.txt"]).expect("stage base");
    git_success(workspace, &["commit", "-m", "recorded base checkpoint"])
        .expect("commit recorded base");
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");
    git_success(workspace, &["reset", "--hard", &root_sha]).expect("rewind task branch");
    fs::write(workspace.join("divergent.txt"), "divergent task history\n").unwrap();
    git_success(workspace, &["add", "divergent.txt"]).expect("stage divergent work");
    git_success(workspace, &["commit", "-m", "divergent task commit"])
        .expect("commit divergent history");
    let divergent_head =
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read divergent head");

    let task = task_with_file("T1", "Divergent task", "divergent.txt", "codex");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_ref": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("non-descendant history is rejected");
    let message = error.to_string();
    assert!(
        message.contains("outside the assigned worktree"),
        "{message}"
    );
    assert!(
        message.contains("Orbit did not inspect, stage, reset, or reconcile any other checkout"),
        "{message}"
    );
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read final divergent head"),
        divergent_head
    );
}

#[test]
fn git_commit_empty_diff_never_stages_or_resets_registered_primary_checkout() {
    let assigned = initialized_git_repo();
    let primary = initialized_git_repo();
    fs::write(
        primary.path().join("README.md"),
        "primary operator change\n",
    )
    .unwrap();
    git_success(primary.path(), &["add", "README.md"]).expect("stage primary change");
    let primary_index_before = git_stdout_bytes(
        primary.path(),
        &["diff", "--cached", "--binary", "HEAD", "--"],
        "snapshot primary index before",
    );

    let task = task_with_file("T1", "Empty task", "src/missing.txt", "codex");
    let host = CommitTestHost::new(vec![task], primary.path().to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": assigned.path().to_string_lossy().to_string(),
    });

    let error = git_commit(&host, &input).expect_err("empty assigned worktree must error");
    let message = error.to_string();
    assert!(
        message.contains("outside the assigned worktree"),
        "{message}"
    );
    assert!(message.contains("attribution may be unknown"), "{message}");
    assert_eq!(
        git_stdout_bytes(
            primary.path(),
            &["diff", "--cached", "--binary", "HEAD", "--"],
            "snapshot primary index after",
        ),
        primary_index_before,
        "git_commit must not stage or reset the registered primary checkout"
    );
}

#[test]
fn git_commit_batch_allows_empty_stage_for_no_diff_expected_task() {
    let temp = initialized_git_repo();
    let workspace = temp.path();

    let mut task = task_with_file("T1", "QA validation", "src/missing.txt", "sonnet");
    task.tags
        .push(orbit_common::types::NO_DIFF_EXPECTED_TAG.to_string());
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    let result = git_commit(&host, &input).expect("exempt empty stage succeeds");
    assert_eq!(result["skipped_no_diff_expected"], json!(true));
    let log = git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count commits");
    assert_eq!(log.trim(), "1");
}

#[test]
fn git_commit_batch_rejects_multiple_tasks() {
    let temp = initialized_git_repo();
    let workspace = temp.path();

    let tasks = vec![
        task_with_file("T1", "Claude task", "src/claude.txt", "claude-opus-4-7"),
        task_with_file("T2", "Gemini task", "src/gemini.txt", "gemini-3.1-pro"),
    ];
    let host = CommitTestHost::new(tasks, workspace.to_path_buf());
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    let error = git_commit(&host, &input).expect_err("multi-task batch is rejected");

    assert!(
        error
            .to_string()
            .contains("commit_batch_changes expected exactly one task")
    );
}
