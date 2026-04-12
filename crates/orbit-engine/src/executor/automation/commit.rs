use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use orbit_types::{OrbitError, Task};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

use super::git::{git_output, git_output_paths, git_success};
use super::input::{canonicalize_existing_dir, input_string_field};

pub(super) fn commit_task_artifact_changes<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = super::parallel::require_run_id(input, "commit_task_artifact_changes")?;
    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;

    let task_ids = completed_task_ids_from_input(input).or_else(|| {
        let batch_tasks = host
            .list_tasks_filtered(None, None, None, Some(batch_id))
            .ok()?;
        let ids: Vec<String> = batch_tasks.into_iter().map(|task| task.id).collect();
        (!ids.is_empty()).then_some(ids)
    });
    let task_ids = task_ids.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "commit_task_artifact_changes: no completed tasks found for batch_id '{batch_id}'"
        ))
    })?;

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
        git_success(&workspace_path, &["commit", "-m", &message])?;
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
    let batch_id = super::parallel::require_run_id(input, "commit_finalize_artifact_changes")?;
    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;

    let changed_files = collect_worktree_changes(&workspace_path)?;
    if changed_files.is_empty() {
        return Ok(json!({}));
    }

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    if batch_tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "commit_finalize_artifact_changes: no tasks found for batch_id '{batch_id}'"
        )));
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

    let message = finalize_commit_message(&affected_tasks);
    git_success(&workspace_path, &["commit", "-m", &message])?;

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
    let batch_id = super::parallel::require_run_id(input, "commit_batch_changes")?;

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    let completed_task_ids: Vec<String> = batch_tasks.iter().map(|t| t.id.clone()).collect();

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "commit_batch_changes: no tasks found for batch_id '{batch_id}'"
        )));
    }

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = staged_changed_files(&workspace_path)?;
    if changed_files.is_empty() {
        git_success(&workspace_path, &["reset", "HEAD"])?;
        return Ok(json!({}));
    }

    let mut task_lines = Vec::new();
    let mut id_labels = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
        task_lines.push(format!("- {}: {}", task_id, task.title.trim()));
        id_labels.push(task_id.clone());
    }
    let ids_joined = id_labels.join(", ");
    let message = format!(
        "feat: parallel batch [{}]\n\nTasks:\n{}",
        ids_joined,
        task_lines.join("\n")
    );

    git_success(&workspace_path, &["commit", "-m", &message])?;
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
            super::parallel::resolve_shared_worktree_path(repo_root, batch_id)
        }
    }
}

fn completed_task_ids_from_input(input: &Value) -> Option<Vec<String>> {
    let items = input.get("completed_task_ids")?.as_array()?;
    let ids = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(ids)
}

fn changed_files_for_task(workspace_path: &Path, task: &Task) -> Result<Vec<String>, OrbitError> {
    let changed_files = collect_worktree_changes(workspace_path)?;
    Ok(filter_changed_files_for_task(
        &changed_files,
        workspace_path,
        task,
    ))
}

fn filter_changed_files_for_task(
    changed_files: &BTreeSet<String>,
    workspace_path: &Path,
    task: &Task,
) -> Vec<String> {
    let scopes = task_scopes(task, workspace_path);
    if scopes.is_empty() {
        return Vec::new();
    }

    changed_files
        .iter()
        .filter(|file| scopes.iter().any(|scope| path_matches_scope(file, scope)))
        .cloned()
        .collect()
}

fn collect_worktree_changes(workspace_path: &Path) -> Result<BTreeSet<String>, OrbitError> {
    let mut files = BTreeSet::new();
    for path in git_output_paths(
        workspace_path,
        &["diff", "--name-only", "-z", "--relative", "HEAD", "--"],
    )? {
        files.insert(path);
    }
    for path in git_output_paths(
        workspace_path,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )? {
        files.insert(path);
    }
    Ok(files)
}

fn task_scopes(task: &Task, workspace_path: &Path) -> Vec<String> {
    task.context_files
        .iter()
        .filter_map(|raw| normalize_task_scope(raw, workspace_path))
        .collect()
}

fn normalize_task_scope(raw: &str, workspace_path: &Path) -> Option<String> {
    let candidate = raw.split('#').next().unwrap_or(raw).trim();
    if candidate.is_empty() {
        return None;
    }

    let path = Path::new(candidate);
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace_path).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    normalize_relative_path(&relative)
}

fn normalize_relative_path(path: &Path) -> Option<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    let value = normalized.to_string_lossy().replace('\\', "/");
    (!value.is_empty()).then_some(value)
}

fn path_matches_scope(path: &str, scope: &str) -> bool {
    path == scope
        || scope == "."
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn task_commit_message(task: &Task) -> String {
    let mut message = format!("[{}] {}", task.id, task.title.trim());
    if let Some(summary) = execution_summary_paragraph(task) {
        message.push_str("\n\n");
        message.push_str(&summary);
    }
    message
}

fn finalize_commit_message(tasks: &[Task]) -> String {
    if tasks.len() == 1 {
        let task = &tasks[0];
        let summary =
            execution_summary_paragraph(task).unwrap_or_else(|| task.title.trim().to_string());
        let subject = single_line_summary(&summary);
        let mut message = format!("fix: {} [{}]", subject, task.id);
        if summary != subject {
            message.push_str("\n\n");
            message.push_str(&summary);
        }
        return message;
    }

    let ids_joined = tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let summaries = tasks
        .iter()
        .map(|task| {
            let summary =
                execution_summary_paragraph(task).unwrap_or_else(|| task.title.trim().to_string());
            format!("- {}: {}", task.id, single_line_summary(&summary))
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("fix: finalize ship batch [{ids_joined}]\n\n{summaries}")
}

fn execution_summary_paragraph(task: &Task) -> Option<String> {
    let section = extract_summary_section(&task.execution_summary)?;
    let paragraph = section
        .lines()
        .map(str::trim)
        .map(|line| {
            line.trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
        })
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let paragraph = paragraph.trim();
    (!paragraph.is_empty()).then_some(paragraph.to_string())
}

fn extract_summary_section(summary: &str) -> Option<String> {
    let mut in_section = false;
    let mut lines = Vec::new();

    for line in summary.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with("## ");
        if trimmed == "## 1. Summary of Changes" || trimmed == "## Summary" {
            in_section = true;
            continue;
        }
        if in_section && is_heading {
            break;
        }
        if in_section {
            lines.push(trimmed.to_string());
        }
    }

    let section = lines.join("\n");
    let section = section.trim();
    (!section.is_empty()).then_some(section.to_string())
}

fn single_line_summary(summary: &str) -> String {
    summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn stage_paths(workspace_path: &Path, files: &[String]) -> Result<(), OrbitError> {
    if files.is_empty() {
        return Ok(());
    }

    let mut args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
    args.extend(files.iter().cloned());
    git_success_dynamic(workspace_path, &args)
}

fn staged_changed_files(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    git_output_paths(
        workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )
}

fn git_success_dynamic(current_dir: &Path, args: &[String]) -> Result<(), OrbitError> {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    git_success(current_dir, &args)
}

fn ensure_named_branch(workspace_path: &Path) -> Result<(), OrbitError> {
    let actual_branch = git_output(workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let actual_branch = actual_branch.trim();
    if actual_branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has detached HEAD; expected a named branch",
            workspace_path.display(),
        )));
    }
    Ok(())
}

fn ensure_no_unmerged_changes(workspace_path: &Path) -> Result<(), OrbitError> {
    let status = git_output(workspace_path, &["status", "--porcelain"])?;
    for line in status.lines() {
        if line.len() < 2 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
            return Err(OrbitError::Execution(format!(
                "task worktree '{}' has unresolved merge conflicts",
                workspace_path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, Job, JobTargetType, OrbitError, OrbitEvent, ReviewThread, Role,
        Task, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::{Value, json};

    use super::{
        commit_finalize_artifact_changes, commit_task_artifact_changes,
        execution_summary_paragraph, task_commit_message,
    };
    use crate::context::{JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskHost};

    struct TestHost {
        repo_root: PathBuf,
        tasks: Vec<Task>,
    }

    impl TaskHost for TestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(self
                .tasks
                .iter()
                .filter(|task| {
                    batch_id.is_none_or(|expected| task.batch_id.as_deref() == Some(expected))
                })
                .cloned()
                .collect())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unreachable!("not used in commit tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unreachable!("not used in commit tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            unreachable!("not used in commit tests")
        }
    }

    impl RuntimeHost for TestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.repo_root.to_string_lossy().to_string())
        }

        fn data_root(&self) -> &Path {
            self.repo_root.as_path()
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            unreachable!("not used in commit tests")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unreachable!("not used in commit tests")
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
            unreachable!("not used in commit tests")
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
            self.repo_root.as_path()
        }
    }

    #[test]
    fn commit_task_artifact_changes_commits_each_completed_task_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        init_repo(&repo_root);

        std::fs::write(repo_root.join("a.txt"), "task one\n").expect("write a");
        std::fs::write(repo_root.join("b.txt"), "task two\n").expect("write b");

        let host = TestHost {
            repo_root: repo_root.clone(),
            tasks: vec![
                sample_task(
                    "T1",
                    "batch-1",
                    TaskType::Feature,
                    vec!["a.txt"],
                    "Add task one flow.",
                ),
                sample_task(
                    "T2",
                    "batch-1",
                    TaskType::Bug,
                    vec!["b.txt"],
                    "Fix task two edge case.",
                ),
                sample_task(
                    "T3",
                    "batch-1",
                    TaskType::Chore,
                    vec!["c.txt"],
                    "Clean up untouched file.",
                ),
            ],
        };

        let result = commit_task_artifact_changes(
            &host,
            &json!({
                "run_id": "batch-1",
                "workspace_path": repo_root.to_string_lossy().to_string(),
                "completed_task_ids": ["T1", "T2", "T3"],
            }),
        )
        .expect("task commits succeed");

        assert_eq!(result.get("committed_task_ids"), Some(&json!(["T1", "T2"])));
        assert_eq!(result.get("skipped_task_ids"), Some(&json!(["T3"])));
        assert_eq!(
            git_stdout(&repo_root, &["log", "--pretty=%s", "-2"]),
            "[T2] Task T2\n[T1] Task T1"
        );
        assert_eq!(git_stdout(&repo_root, &["status", "--short"]), "");
    }

    #[test]
    fn commit_finalize_artifact_changes_commits_only_task_scoped_leftovers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        init_repo(&repo_root);

        std::fs::write(repo_root.join("a.txt"), "finalize a\n").expect("write a");
        std::fs::write(repo_root.join("b.txt"), "finalize b\n").expect("write b");
        std::fs::write(repo_root.join("orphan.txt"), "leftover\n").expect("write orphan");

        let host = TestHost {
            repo_root: repo_root.clone(),
            tasks: vec![
                sample_task(
                    "T1",
                    "batch-1",
                    TaskType::Feature,
                    vec!["a.txt"],
                    "Updated task one after integration review.",
                ),
                sample_task(
                    "T2",
                    "batch-1",
                    TaskType::Bug,
                    vec!["b.txt"],
                    "Fixed task two after shared verification.",
                ),
            ],
        };

        let result = commit_finalize_artifact_changes(
            &host,
            &json!({
                "run_id": "batch-1",
                "workspace_path": repo_root.to_string_lossy().to_string(),
            }),
        )
        .expect("finalize commit succeeds");

        assert_eq!(result.get("committed_task_ids"), Some(&json!(["T1", "T2"])));
        assert_eq!(
            git_stdout(&repo_root, &["log", "-1", "--pretty=%s"]),
            "fix: finalize ship batch [T1, T2]"
        );
        assert_eq!(
            git_stdout(&repo_root, &["status", "--short"]),
            "?? orphan.txt"
        );
    }

    #[test]
    fn execution_summary_paragraph_extracts_summary_section() {
        let task = sample_task(
            "T1",
            "batch-1",
            TaskType::Feature,
            vec!["a.txt"],
            "Added retry logic to batch dispatch.",
        );

        assert_eq!(
            execution_summary_paragraph(&task).as_deref(),
            Some("Added retry logic to batch dispatch.")
        );
    }

    #[test]
    fn task_commit_message_uses_title_subject_and_summary_body() {
        let task = sample_task(
            "T1",
            "batch-1",
            TaskType::Feature,
            vec!["a.txt"],
            "Added retry logic to batch dispatch.",
        );

        assert_eq!(
            task_commit_message(&task),
            "[T1] Task T1\n\nAdded retry logic to batch dispatch."
        );
    }

    fn sample_task(
        id: &str,
        batch_id: &str,
        task_type: TaskType,
        context_files: Vec<&str>,
        summary: &str,
    ) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            parent_id: None,
            title: format!("Task {id}"),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            plan: "1. Do the thing".to_string(),
            execution_summary: format!(
                "## Status\nsuccess\n\n## 1. Summary of Changes\n{summary}\n\n## 2. Strategic Decisions\n- None"
            ),
            context_files: context_files.into_iter().map(str::to_string).collect(),
            workspace_path: Some("/repo".to_string()),
            repo_root: Some("/repo".to_string()),
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::Review,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: Some(batch_id.to_string()),
            comments: vec![],
            history: vec![],
            review_threads: Vec::<ReviewThread>::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn init_repo(repo_root: &Path) {
        std::fs::create_dir_all(repo_root).expect("create repo");
        run_git(repo_root, &["init", "-b", "main"]);
        run_git(repo_root, &["config", "user.name", "Orbit Tests"]);
        run_git(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        );
        run_git(repo_root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo_root.join("a.txt"), "base a\n").expect("write a");
        std::fs::write(repo_root.join("b.txt"), "base b\n").expect("write b");
        run_git(repo_root, &["add", "a.txt", "b.txt"]);
        run_git(repo_root, &["commit", "-m", "initial"]);
    }

    fn run_git(current_dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(current_dir)
            .args(args)
            .output()
            .expect("git output");
        assert!(
            output.status.success(),
            "git {} failed in '{}': stdout={} stderr={}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(current_dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(current_dir)
            .args(args)
            .output()
            .expect("git output");
        assert!(
            output.status.success(),
            "git {} failed in '{}': stdout={} stderr={}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }
}
