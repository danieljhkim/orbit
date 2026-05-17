mod author;
mod git_ops;
mod message;
mod scope;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::{Value, json};

#[cfg(test)]
use orbit_common::types::Task;

use crate::context::{RuntimeHost, TaskHost};

use super::super::input::{canonicalize_existing_dir, input_string_field, required_job_run_id};
use super::git::git_success;
use author::{append_co_author_trailers, commit_author_for_tasks, git_author_for_task};
use git_ops::{
    ensure_named_branch, ensure_no_unmerged_changes, git_commit_with_identity, stage_paths,
    staged_changed_files,
};
use message::{finalize_commit_message, task_commit_message};
use scope::{changed_files_for_task, collect_worktree_changes, filter_changed_files_for_task};

#[cfg(test)]
use scope::{normalize_task_scope, path_matches_scope};

pub(in crate::executor::automation) fn git_commit<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("all");
    match scope {
        "per_task" => commit_task_artifact_changes(host, input),
        "per_task_finalize" => commit_finalize_artifact_changes(host, input),
        "all" => commit_batch_changes(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "git_commit: unknown scope '{other}'; expected per_task, per_task_finalize, or all"
        ))),
    }
}

pub(super) fn commit_task_artifact_changes<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_task_artifact_changes")?;
    let explicit_completed_task_ids = completed_task_ids_field(input);
    if explicit_completed_task_ids
        .as_ref()
        .is_some_and(|task_ids| task_ids.is_empty())
    {
        return Ok(json!({
            "committed_task_ids": [],
            "skipped_task_ids": [],
        }));
    }

    let fallback_batch_tasks = if explicit_completed_task_ids.is_none() {
        Some(host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?)
    } else {
        None
    };
    if fallback_batch_tasks
        .as_ref()
        .is_some_and(|batch_tasks| batch_tasks.is_empty())
    {
        return Ok(json!({
            "committed_task_ids": [],
            "skipped_task_ids": [],
        }));
    }

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;

    let task_ids = match explicit_completed_task_ids {
        Some(task_ids) => task_ids,
        None => fallback_batch_tasks
            .unwrap_or_default()
            .into_iter()
            .map(|task| task.id)
            .collect(),
    };

    let mut committed_task_ids = Vec::new();
    let mut skipped_task_ids = Vec::new();

    for task_id in task_ids {
        let task = host.get_task(&task_id)?;
        let changed_files = changed_files_for_task(&workspace_path, &task)?;
        if changed_files.is_empty() {
            skipped_task_ids.push(task_id);
            continue;
        }

        stage_paths(&workspace_path, &changed_files)?;
        let staged_files = staged_changed_files(&workspace_path)?;
        if staged_files.is_empty() {
            skipped_task_ids.push(task.id);
            continue;
        }

        let message = task_commit_message(&task);
        let author = git_author_for_task(&task);
        git_commit_with_identity(&workspace_path, &message, author.as_ref())?;
        committed_task_ids.push(task.id);
    }

    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "committed_task_ids": committed_task_ids,
        "skipped_task_ids": skipped_task_ids,
    }))
}

pub(super) fn commit_finalize_artifact_changes<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_finalize_artifact_changes")?;
    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    if batch_tasks.is_empty() {
        return Ok(json!({}));
    }

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;

    let changed_files = collect_worktree_changes(&workspace_path)?;
    if changed_files.is_empty() {
        return Ok(json!({}));
    }

    let mut affected_tasks = Vec::new();
    let mut files_to_commit = BTreeSet::new();
    for task in batch_tasks {
        let task_files = filter_changed_files_for_task(&changed_files, &workspace_path, &task);
        if task_files.is_empty() {
            continue;
        }
        files_to_commit.extend(task_files);
        affected_tasks.push(task);
    }

    if affected_tasks.is_empty() {
        return Ok(json!({}));
    }

    let files_to_commit: Vec<String> = files_to_commit.into_iter().collect();
    stage_paths(&workspace_path, &files_to_commit)?;
    let staged_files = staged_changed_files(&workspace_path)?;
    if staged_files.is_empty() {
        return Ok(json!({}));
    }

    let mut message = finalize_commit_message(&affected_tasks);
    let (author, coauthors) = commit_author_for_tasks(&affected_tasks);
    append_co_author_trailers(&mut message, &coauthors);
    git_commit_with_identity(&workspace_path, &message, author.as_ref())?;

    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "committed_task_ids": affected_tasks.into_iter().map(|task| task.id).collect::<Vec<_>>(),
        "committed_files": staged_files,
    }))
}

pub(super) fn commit_batch_changes<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_batch_changes")?;
    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    if batch_tasks.is_empty() {
        return Ok(json!({}));
    }

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = staged_changed_files(&workspace_path)?;
    if changed_files.is_empty() {
        git_success(&workspace_path, &["reset", "HEAD"])?;
        return Ok(json!({}));
    }

    let mut task_lines = Vec::new();
    let mut id_labels = Vec::new();
    for task in &batch_tasks {
        task_lines.push(format!("- {}: {}", task.id, task.title.trim()));
        id_labels.push(task.id.clone());
    }
    let ids_joined = id_labels.join(", ");
    let mut message = format!(
        "feat: parallel batch [{}]\n\nTasks:\n{}",
        ids_joined,
        task_lines.join("\n")
    );
    let (author, coauthors) = commit_author_for_tasks(&batch_tasks);
    append_co_author_trailers(&mut message, &coauthors);

    git_commit_with_identity(&workspace_path, &message, author.as_ref())?;
    Ok(json!({}))
}

fn resolve_workspace_path<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
    batch_id: &str,
) -> Result<PathBuf, OrbitError> {
    match input_string_field(input, "workspace_path") {
        Some(ws) => canonicalize_existing_dir(&ws, "workspace_path"),
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            super::worktree::resolve_shared_worktree_path(repo_root, batch_id)
        }
    }
}

fn completed_task_ids_field(input: &Value) -> Option<Vec<String>> {
    let items = input.get("completed_task_ids")?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use chrono::Utc;
    use orbit_common::types::{
        Activity, Job, JobTargetType, NotFoundKind, OrbitEvent, Role, TaskArtifact, TaskPriority,
        TaskStatus, TaskType,
    };
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::context::{
        JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskReadHost, TaskWriteHost,
    };
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::super::git::git_output;
    use super::*;

    struct CommitTestHost {
        tasks: Vec<Task>,
        repo_root: PathBuf,
        data_root: PathBuf,
        scoreboard_dir: PathBuf,
        registry: ActivityExecutorRegistry,
    }

    impl CommitTestHost {
        fn new(tasks: Vec<Task>, repo_root: PathBuf) -> Self {
            let data_root = repo_root.join(".orbit-test-data");
            let scoreboard_dir = data_root.join("scoreboard");
            Self {
                tasks,
                repo_root,
                data_root,
                scoreboard_dir,
                registry: ActivityExecutorRegistry::default(),
            }
        }
    }

    impl TaskReadHost for CommitTestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))
        }

        fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            status: Option<TaskStatus>,
            priority: Option<TaskPriority>,
            parent_id: Option<&str>,
            batch_id: Option<&str>,
            external_ref: Option<&orbit_common::types::ExternalRef>,
            has_external_ref_system: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(self
                .tasks
                .iter()
                .filter(|task| status.is_none_or(|status| task.status == status))
                .filter(|task| priority.is_none_or(|priority| task.priority == priority))
                .filter(|task| {
                    parent_id.is_none_or(|parent_id| task.parent_id() == Some(parent_id))
                })
                .filter(|task| {
                    batch_id.is_none_or(|batch_id| task.job_run_id.as_deref() == Some(batch_id))
                })
                .filter(|task| {
                    external_ref.is_none_or(|external_ref| {
                        task.external_refs.iter().any(|candidate| {
                            candidate.system == external_ref.system
                                && candidate.id == external_ref.id
                        })
                    })
                })
                .filter(|task| {
                    has_external_ref_system.is_none_or(|system| {
                        task.external_refs
                            .iter()
                            .any(|candidate| candidate.system == system)
                    })
                })
                .cloned()
                .collect())
        }
    }

    impl TaskWriteHost for CommitTestHost {
        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "start_task is not needed by commit tests".to_string(),
            ))
        }

        fn admit_task_for_workflow(
            &self,
            _task_id: &str,
            _workflow: &str,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "admit_task_for_workflow is not needed by commit tests".to_string(),
            ))
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
            _agent: Option<String>,
            _model: Option<String>,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "update_task_from_activity is not needed by commit tests".to_string(),
            ))
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            Err(OrbitError::Execution(
                "apply_task_automation_update is not needed by commit tests".to_string(),
            ))
        }
    }

    impl RuntimeHost for CommitTestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.repo_root.to_string_lossy().to_string())
        }

        fn data_root(&self) -> &Path {
            &self.data_root
        }

        fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
            &self.registry
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            Err(OrbitError::Execution(
                "run_job_now_with_input_debug is not needed by commit tests".to_string(),
            ))
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            Err(OrbitError::Execution(
                "validate_activity_target_exists is not needed by commit tests".to_string(),
            ))
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            Ok(None)
        }

        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            Err(OrbitError::Execution(
                "run_tool_with_context_and_role is not needed by commit tests".to_string(),
            ))
        }

        fn maybe_create_failure_task(
            &self,
            _job_id: &str,
            _run_id: &str,
            _error_code: &str,
            _error_message: &str,
            _agent: Option<&str>,
            _model: Option<&str>,
        ) -> Result<(), OrbitError> {
            Ok(())
        }

        fn scoring_enabled(&self) -> bool {
            false
        }

        fn graph_editing(&self) -> bool {
            false
        }

        fn scoreboard_dir(&self) -> &Path {
            &self.scoreboard_dir
        }
    }

    #[test]
    fn normalize_task_scope_uses_selector_anchor_paths() {
        let temp = tempdir().unwrap();
        let workspace = temp.path();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/lib.rs"), "pub fn run() {}\n").unwrap();

        assert_eq!(
            normalize_task_scope("symbol:src/lib.rs#run:function", workspace).as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(
            normalize_task_scope("dir:src", workspace).as_deref(),
            Some("src")
        );
        assert_eq!(
            normalize_task_scope(&workspace.join("src/lib.rs").to_string_lossy(), workspace)
                .as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn path_matches_scope_handles_directory_scopes() {
        assert!(path_matches_scope("src/lib.rs", "src"));
        assert!(path_matches_scope("src/lib.rs", "src/lib.rs"));
        assert!(!path_matches_scope("tests/lib.rs", "src"));
    }

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

            let actual_author = git_output(workspace, &["log", "-1", "--format=%an <%ae>"])
                .expect("read git author");
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
    fn git_commit_mixed_implementer_batch_uses_aggregate_identity_with_trailers() {
        let temp = initialized_git_repo();
        let workspace = temp.path();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/claude.txt"), "claude work\n").unwrap();
        fs::write(workspace.join("src/gemini.txt"), "gemini work\n").unwrap();

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

        let local_user_config_before = local_user_config_snapshot(workspace);

        git_commit(&host, &input).expect("git_commit succeeds");

        let actual_author =
            git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read git author");
        let actual_committer = git_output(workspace, &["log", "-1", "--format=%cn <%ce>"])
            .expect("read git committer");
        let body = git_output(workspace, &["log", "-1", "--format=%B"]).expect("read git body");
        assert_eq!(actual_author, "orbit <orbit@orbit.local>");
        assert_eq!(actual_committer, "orbit <orbit@orbit.local>");
        assert!(body.contains("Co-Authored-By: claude <claude@orbit.local>"));
        assert!(body.contains("Co-Authored-By: gemini <gemini@orbit.local>"));
        assert_eq!(
            local_user_config_snapshot(workspace),
            local_user_config_before
        );
    }

    fn initialized_git_repo() -> tempfile::TempDir {
        let temp = tempdir().unwrap();
        let repo = temp.path();
        git_success(repo, &["init"]).expect("git init");
        git_success(repo, &["config", "user.name", "Local User"]).expect("config user.name");
        git_success(repo, &["config", "user.email", "local@example.test"])
            .expect("config user.email");
        fs::write(repo.join("README.md"), "base\n").unwrap();
        git_success(repo, &["add", "README.md"]).expect("git add");
        git_success(repo, &["commit", "-m", "initial commit"]).expect("initial commit");
        temp
    }

    fn initialized_git_repo_without_local_user_config() -> tempfile::TempDir {
        let temp = tempdir().unwrap();
        let repo = temp.path();
        git_success(repo, &["init"]).expect("git init");
        fs::write(repo.join("README.md"), "base\n").unwrap();
        git_success(repo, &["add", "README.md"]).expect("git add");
        git_success(
            repo,
            &[
                "-c",
                "user.name=Initial User",
                "-c",
                "user.email=initial@example.test",
                "commit",
                "-m",
                "initial commit",
            ],
        )
        .expect("initial commit");
        assert_eq!(
            local_user_config_snapshot(repo),
            CommandSnapshot {
                code: Some(1),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        );
        temp
    }

    #[derive(Debug, Eq, PartialEq)]
    struct CommandSnapshot {
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    fn local_user_config_snapshot(repo: &Path) -> CommandSnapshot {
        git_command_snapshot(repo, &["config", "--local", "--get-regexp", "^user\\."])
    }

    fn git_stdout_bytes(repo: &Path, args: &[&str], context: &str) -> Vec<u8> {
        let snapshot = git_command_snapshot(repo, args);
        assert_eq!(
            snapshot.code,
            Some(0),
            "{context}: stderr={}",
            String::from_utf8_lossy(&snapshot.stderr)
        );
        snapshot.stdout
    }

    fn git_command_snapshot(repo: &Path, args: &[&str]) -> CommandSnapshot {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("run git command");
        CommandSnapshot {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }

    fn task_with_file(id: &str, title: &str, path: &str, implemented_by: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![format!("file:{path}")],
            created_by: None,
            planned_by: None,
            implemented_by: Some(implemented_by.to_string()),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: Some("batch-1".to_string()),
            crew: None,
            created_at: now,
            updated_at: now,
        }
    }
}
