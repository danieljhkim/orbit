use std::fs;

use orbit_common::types::TaskType;
use serde_json::json;

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::git_output;

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
        "## Summary\n- Fixed deterministic batch commit messages.\n\n## Validation\n- cargo test"
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
