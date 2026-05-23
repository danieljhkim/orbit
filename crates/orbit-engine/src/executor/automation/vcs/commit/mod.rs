mod author;
mod git_ops;
mod message;
mod scope;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

use super::super::input::{canonicalize_existing_dir, input_string_field, required_job_run_id};
use super::git::git_success;
use author::{append_co_author_trailers, commit_author_for_tasks, git_author_for_task};
use git_ops::{
    ensure_named_branch, ensure_no_unmerged_changes, git_commit_with_identity, stage_paths,
    staged_changed_files,
};
use message::{batch_commit_message, finalize_commit_message, task_commit_message};
use scope::{changed_files_for_task, collect_worktree_changes, filter_changed_files_for_task};

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
    let [task] = batch_tasks.as_slice() else {
        return Err(OrbitError::InvalidInput(format!(
            "commit_batch_changes expected exactly one task for job_run_id '{batch_id}', got {}",
            batch_tasks.len()
        )));
    };

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = staged_changed_files(&workspace_path)?;
    if changed_files.is_empty() {
        git_success(&workspace_path, &["reset", "HEAD"])?;
        return Ok(json!({}));
    }

    let message = batch_commit_message(task);
    let author = git_author_for_task(task);

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
mod tests;
