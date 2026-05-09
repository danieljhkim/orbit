use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use orbit_common::types::{OrbitError, Task, infer_agent_family_from_model};
use orbit_common::utility::selector::anchor_path;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

use super::git::{git_output, git_output_paths, git_success};
use super::input::{canonicalize_existing_dir, input_string_field, required_batch_id};

pub(super) fn git_commit<H: TaskHost + RuntimeHost + ?Sized>(
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
    let batch_id = required_batch_id(input, "commit_task_artifact_changes")?;
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
        git_commit_with_author(&workspace_path, &message, author.as_ref())?;
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
    let batch_id = required_batch_id(input, "commit_finalize_artifact_changes")?;
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
    git_commit_with_author(&workspace_path, &message, author.as_ref())?;

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
    let batch_id = required_batch_id(input, "commit_batch_changes")?;
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

    git_commit_with_author(&workspace_path, &message, author.as_ref())?;
    Ok(json!({}))
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GitAuthor {
    name: String,
    email: String,
}

impl GitAuthor {
    fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
        }
    }

    fn spec(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }

    fn trailer(&self) -> String {
        format!("Co-Authored-By: {}", self.spec())
    }
}

fn git_author_for_task(task: &Task) -> Option<GitAuthor> {
    git_author_for_implemented_by(task.implemented_by.as_deref())
}

fn commit_author_for_tasks(tasks: &[Task]) -> (Option<GitAuthor>, Vec<GitAuthor>) {
    let authors = tasks
        .iter()
        .filter_map(git_author_for_task)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    match authors.as_slice() {
        [] => (None, Vec::new()),
        [author] => (Some(author.clone()), Vec::new()),
        _ => (Some(GitAuthor::new("orbit", "orbit@orbit.local")), authors),
    }
}

fn append_co_author_trailers(message: &mut String, coauthors: &[GitAuthor]) {
    if coauthors.is_empty() {
        return;
    }

    message.push_str("\n\n");
    message.push_str(
        &coauthors
            .iter()
            .map(GitAuthor::trailer)
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

fn git_author_for_implemented_by(implemented_by: Option<&str>) -> Option<GitAuthor> {
    let implemented_by = implemented_by?.trim();
    if implemented_by.is_empty() {
        return None;
    }

    match implementer_family(implemented_by).as_deref() {
        Some("claude") => Some(GitAuthor::new("claude", "claude@orbit.local")),
        Some("gemini") => Some(GitAuthor::new("gemini", "gemini@orbit.local")),
        Some("codex") => Some(GitAuthor::new("codex", "codex@openai.com")),
        _ => {
            let slug = author_slug(implemented_by);
            Some(GitAuthor::new(slug.clone(), format!("{slug}@orbit.local")))
        }
    }
}

fn implementer_family(implemented_by: &str) -> Option<String> {
    let lower = implemented_by.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }

    let model_hint = lower
        .rsplit_once(" / ")
        .map(|(_, model)| model.trim())
        .unwrap_or(lower.as_str());

    infer_agent_family_from_model(model_hint)
        .or_else(|| {
            if model_hint.starts_with("o4") {
                Some("codex".to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            if lower.starts_with("codex") || lower.starts_with("openai") {
                Some("codex".to_string())
            } else if lower.starts_with("claude") || lower.contains("/claude") {
                Some("claude".to_string())
            } else if lower.starts_with("gemini") || lower.contains("/gemini") {
                Some("gemini".to_string())
            } else {
                None
            }
        })
}

fn author_slug(label: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in label.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug
    }
}

fn git_commit_with_author(
    workspace_path: &Path,
    message: &str,
    author: Option<&GitAuthor>,
) -> Result<(), OrbitError> {
    let mut args = vec!["commit".to_string()];
    if let Some(author) = author {
        args.push("--author".to_string());
        args.push(author.spec());
    }
    args.extend(["-m".to_string(), message.to_string()]);
    git_success_dynamic(workspace_path, &args)
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
    let anchor = anchor_path(raw).ok()?;
    let relative = if anchor.is_absolute() {
        anchor.strip_prefix(workspace_path).ok()?.to_path_buf()
    } else {
        anchor
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
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use orbit_common::types::{
        Activity, Job, JobTargetType, OrbitEvent, Role, TaskArtifact, TaskPriority, TaskStatus,
        TaskType,
    };
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use crate::context::{
        JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskReadHost, TaskWriteHost,
    };
    use crate::executor::registry::ActivityExecutorRegistry;

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
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
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
                    parent_id.is_none_or(|parent_id| task.parent_id.as_deref() == Some(parent_id))
                })
                .filter(|task| {
                    batch_id.is_none_or(|batch_id| task.batch_id.as_deref() == Some(batch_id))
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
    fn git_commit_uses_task_implemented_by_as_author() {
        let cases = [
            ("claude-opus-4-7", "claude <claude@orbit.local>"),
            ("gemini-3.1-pro", "gemini <gemini@orbit.local>"),
            ("gpt-5.5", "codex <codex@openai.com>"),
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
                "batch_id": "batch-1",
                "workspace_path": workspace.to_string_lossy().to_string(),
                "completed_task_ids": ["T1"],
            });

            let user_name_before = git_output(workspace, &["config", "--get", "user.name"])
                .expect("read git user.name before");
            let user_email_before = git_output(workspace, &["config", "--get", "user.email"])
                .expect("read git user.email before");

            git_commit(&host, &input).expect("git_commit succeeds");

            let actual_author = git_output(workspace, &["log", "-1", "--format=%an <%ae>"])
                .expect("read git author");
            assert_eq!(actual_author, expected_author);
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
        }
    }

    #[test]
    fn mixed_implementer_batch_commit_uses_aggregate_author_with_trailers() {
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
            "batch_id": "batch-1",
            "workspace_path": workspace.to_string_lossy().to_string(),
        });

        git_commit(&host, &input).expect("git_commit succeeds");

        let actual_author =
            git_output(workspace, &["log", "-1", "--format=%an <%ae>"]).expect("read git author");
        let body = git_output(workspace, &["log", "-1", "--format=%B"]).expect("read git body");
        assert_eq!(actual_author, "orbit <orbit@orbit.local>");
        assert!(body.contains("Co-Authored-By: claude <claude@orbit.local>"));
        assert!(body.contains("Co-Authored-By: gemini <gemini@orbit.local>"));
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

    fn task_with_file(id: &str, title: &str, path: &str, implemented_by: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![format!("file:{path}")],
            workspace_path: None,
            repo_root: None,
            created_by: None,
            planned_by: None,
            implemented_by: Some(implemented_by.to_string()),
            agent: None,
            model: None,
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            pr_status: None,
            external_refs: Vec::new(),
            source_task_id: None,
            batch_id: Some("batch-1".to_string()),
            comments: Vec::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}
