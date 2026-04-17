use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use orbit_types::{OrbitError, Task};
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
    let batch_id = required_batch_id(input, "commit_finalize_artifact_changes")?;
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
    let batch_id = required_batch_id(input, "commit_batch_changes")?;

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
