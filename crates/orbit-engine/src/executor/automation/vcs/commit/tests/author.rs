use std::fs;
use std::os::unix::fs::PermissionsExt;

use orbit_common::types::{OrbitError, TaskType};
use serde_json::json;

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::{git_output, git_success};

#[test]
fn git_commit_uses_resolved_model_author_without_mutating_local_human_config() {
    let cases = [("opus", "claude-opus-5"), ("terra", "gpt-5.6-terra")];
    let mut author_names = Vec::new();

    for (crew_alias, resolved_model) in cases {
        let temp = initialized_git_repo();
        let workspace = temp.path();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(
            workspace.join("src/task.txt"),
            format!("implemented by {crew_alias}\n"),
        )
        .unwrap();

        let task = task_with_file("T1", "Implement one task", "src/task.txt", crew_alias);
        let host = CommitTestHost::new(vec![task], workspace.to_path_buf())
            .with_crew_model(resolved_model);
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
        let author_name =
            git_output(workspace, &["log", "-1", "--format=%an"]).expect("read git author name");
        let actual_committer = git_output(workspace, &["log", "-1", "--format=%cn <%ce>"])
            .expect("read git committer");
        assert_eq!(
            actual_author,
            format!("orbit[{resolved_model}] <agent@orbit.invalid>")
        );
        assert_eq!(actual_committer, "orbit <orbit@orbit.local>");
        assert_ne!(
            actual_author,
            format!("orbit[{crew_alias}] <agent@orbit.invalid>"),
            "the author must use the resolved model, not crew alias '{crew_alias}'"
        );
        author_names.push(author_name);
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
    assert_eq!(
        author_names,
        ["orbit[claude-opus-5]", "orbit[gpt-5.6-terra]"],
        "plain git log author names distinguish the two resolved models"
    );
}

#[test]
fn git_commit_without_resolved_model_uses_generic_fallback_and_no_local_config() {
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
    assert_eq!(actual_author, "orbit <orbit@orbit.local>");
    assert_eq!(actual_committer, "orbit <orbit@orbit.local>");
    assert_eq!(
        local_user_config_snapshot(workspace),
        local_user_config_before
    );
}

#[test]
fn git_commit_treats_bare_configured_model_as_opaque() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/task.txt"), "bare model config\n").unwrap();

    let task = task_with_file("T1", "Implement one task", "src/task.txt", "claude");
    let host = CommitTestHost::new(vec![task], workspace.to_path_buf()).with_crew_model("opus");
    let input = json!({
        "scope": "per_task",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "completed_task_ids": ["T1"],
    });

    git_commit(&host, &input).expect("bare configured model remains accepted");

    assert_eq!(
        git_output(workspace, &["log", "-1", "--format=%an"]).expect("read model author"),
        "orbit[opus]"
    );
}

#[test]
fn git_commit_gives_hook_the_same_model_and_preserves_other_trailers() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    install_telemetry_hook(workspace);
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/one.txt"), "one\n").unwrap();
    fs::write(workspace.join("src/two.txt"), "two\n").unwrap();

    let tasks = vec![
        task_with_file("T1", "Implement one task", "src/one.txt", "claude-opus-5"),
        task_with_file("T2", "Implement another", "src/two.txt", "gpt-5.6-terra"),
    ];
    let host = CommitTestHost::new(tasks, workspace.to_path_buf()).with_crew_model("claude-opus-5");
    let input = json!({
        "scope": "per_task_finalize",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    });

    git_commit(&host, &input).expect("git_commit succeeds with telemetry hook");

    let author = git_output(workspace, &["log", "-1", "--format=%an"]).expect("read model author");
    let body = git_output(workspace, &["log", "-1", "--format=%B"]).expect("read commit body");
    assert_eq!(author, "orbit[claude-opus-5]");
    assert!(body.contains("Agent-Run: inherited-run"), "{body}");
    assert!(body.contains("Agent-Model: claude-opus-5"), "{body}");
    assert!(body.contains("Agent-Task: T1"), "{body}");
    assert!(
        body.contains("Co-Authored-By: claude <claude@orbit.local>"),
        "{body}"
    );
    assert!(
        body.contains("Co-Authored-By: codex <codex@orbit.local>"),
        "{body}"
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
    let host =
        CommitTestHost::new(vec![task], workspace.to_path_buf()).with_crew_model("claude-opus-5");
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
    assert_eq!(actual_author, "orbit[claude-opus-5] <agent@orbit.invalid>");
    assert_eq!(actual_committer, "orbit <orbit@orbit.local>");
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
        "base_sha": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("empty stage must error");
    let OrbitError::Execution(message) = error else {
        panic!("expected execution error, got: {error}");
    };
    // ORB-10380 reconciliation: the assertion still pins the *exact* message,
    // but the message changed on purpose. The old text was shared verbatim with
    // the ancestry gate and guessed at causes ("may have been written outside
    // the assigned worktree, or attribution may be unknown") that this branch
    // never observed. The empty-stage diagnostic now reports only measured
    // state; the intent this test protects — a clean worktree errors, never
    // silently succeeds — is unchanged.
    assert_eq!(
        message,
        format!(
            "commit_batch_changes: nothing to commit for task 'T1' in worktree '{}'. \
             Observed after `git add --all`: 0 staged, 0 unstaged, 0 untracked file(s); \
             HEAD {base_sha}; pinned base {base_sha}; 0 commit(s) above it. Orbit did not \
             inspect, stage, reset, or reconcile any other checkout",
            workspace.display()
        )
    );

    // No commit was created beyond the repo's initial commit.
    let log = git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count commits");
    assert_eq!(log.trim(), "1", "no commit should have been created");
}

#[test]
fn git_commit_batch_adopts_implement_authored_commits_without_rewriting_them() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    let base_sha = git_output(workspace, &["rev-parse", "HEAD"]).expect("read base checkpoint");

    fs::write(workspace.join("README.md"), "implementation\n").unwrap();
    git_success(workspace, &["add", "README.md"]).expect("stage implementation");
    git_success(
        workspace,
        &[
            "-c",
            "user.name=Implement Agent",
            "-c",
            "user.email=implement@example.test",
            "commit",
            "-m",
            "auto-commit",
            "-m",
            "Agent-Run: batch-1\nAgent-Task: T1",
        ],
    )
    .expect("author attributed implementation commit");

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
        "base_sha": base_sha,
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
        git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read retained author"),
        "Implement Agent <implement@example.test>"
    );
}

#[test]
fn git_commit_batch_rejects_dirty_residue_above_an_attributed_commit() {
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
            "auto-commit",
            "-m",
            "Agent-Run: batch-1\nAgent-Task: T1",
        ],
    )
    .expect("author implement commit");
    let authored_sha =
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read implement commit");
    fs::write(workspace.join("residue.txt"), "dirty residue\n").unwrap();

    let task = task_with_file("T1", "Mixed history", "residue.txt", "codex");
    let host =
        CommitTestHost::new(vec![task], workspace.to_path_buf()).with_crew_model("gpt-5.6-terra");
    let input = json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
        "base_sha": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("mixed commit-plus-dirty state is ambiguous");
    assert!(
        error.to_string().contains("worktree_content_conflict"),
        "{error}"
    );
    assert!(error.to_string().contains("residue.txt"), "{error}");
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read unchanged head"),
        authored_sha,
        "the conflict must not create a residue commit"
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
        fs::read_to_string(workspace.join("residue.txt")).expect("dirty residue remains"),
        "dirty residue\n"
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
        "base_sha": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("other branch commit is not adopted");
    let message = error.to_string();
    assert!(message.contains("nothing to commit"), "{message}");
    assert!(
        message.contains("0 commit(s) above it"),
        "the diagnostic reports the observed count, not a guess: {message}"
    );
    assert!(
        message.contains("Orbit did not inspect, stage, reset, or reconcile any other checkout"),
        "{message}"
    );
    assert_eq!(
        git_output(workspace, &["rev-list", "--count", "HEAD"]).expect("count assigned commits"),
        "1"
    );
}

#[test]
fn git_commit_batch_rejects_history_rewritten_below_the_pinned_base() {
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
        "base_sha": base_sha,
    });

    let error = git_commit(&host, &input).expect_err("rewritten history is never adopted");
    let message = error.to_string();
    assert!(message.contains("worktree_history_rewritten"), "{message}");
    assert!(message.contains(&base_sha), "{message}");
    assert!(message.contains(&divergent_head), "{message}");
    assert_eq!(
        git_output(workspace, &["rev-parse", "HEAD"]).expect("read final divergent head"),
        divergent_head,
        "the failure must not rewrite the task branch"
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
        message.contains("nothing to commit for task 'T1'"),
        "{message}"
    );
    assert!(
        !message.contains(&primary.path().display().to_string()),
        "the diagnostic reports the assigned worktree, never the primary checkout: {message}"
    );
    assert!(
        message.contains("Orbit did not inspect, stage, reset, or reconcile any other checkout"),
        "{message}"
    );
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

fn install_telemetry_hook(workspace: &std::path::Path) {
    let hook = workspace
        .join(".git")
        .join("orbit-test-empty-hooks")
        .join("prepare-commit-msg");
    fs::write(
        &hook,
        "#!/bin/sh\n\
         printf '\\nAgent-Run: inherited-run\\nAgent-Model: %s\\nAgent-Task: T1\\n' \
         \"${AGENT_MODEL:-}\" >> \"$1\"\n",
    )
    .expect("write prepare-commit-msg fixture");
    let mut permissions = fs::metadata(&hook)
        .expect("read hook metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).expect("make hook executable");
}
