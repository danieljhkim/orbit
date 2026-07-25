mod author;
mod git_ops;
mod message;
mod scope;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{NO_DIFF_EXPECTED_TAG, OrbitError};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

use super::super::input::{canonicalize_existing_dir, input_string_field, required_job_run_id};
use super::git::{git_command_success, git_output, git_success};
use super::handoff::reject_failed_delivery;
use author::{append_co_author_trailers, commit_author_for_tasks};
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
    let resolved_model = host.resolved_crew_model(batch_id)?;

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
        git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;
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
    let (_, coauthors) = commit_author_for_tasks(&affected_tasks);
    append_co_author_trailers(&mut message, &coauthors);
    let resolved_model = host.resolved_crew_model(batch_id)?;
    git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;

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

    // ORB-10313: fail closed on the durable execution outcome before resolving
    // the delivery checkout, staging files, mutating the index, or committing.
    reject_failed_delivery(task)?;

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;

    ensure_no_unmerged_changes(&workspace_path)?;
    let (base_sha, mut commit_shas) = existing_batch_commits(&workspace_path, input, &task.id)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = staged_changed_files(&workspace_path)?;
    if changed_files.is_empty() {
        if !commit_shas.is_empty() {
            let commit_sha = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
            let mut result = json!({
                "phase": "commit",
                "decision": "adopted_existing_commits",
                "committed": false,
                "adopted_commits": true,
                "commit_sha": commit_sha.trim(),
                "commit_shas": commit_shas,
                "job_run_id": batch_id,
                "skipped_no_diff_expected": false,
                "task_id": task.id,
            });
            if let Some(base_sha) = base_sha {
                result["base_sha"] = json!(base_sha);
            }
            return Ok(result);
        }

        git_success(&workspace_path, &["reset", "HEAD"])?;
        // ADR-0219: explicit side-effect-only tasks bypass the empty-diff gate.
        if task.tags.iter().any(|tag| tag == NO_DIFF_EXPECTED_TAG) {
            return Ok(json!({
                "phase": "commit",
                "decision": "skipped_no_diff_expected",
                "committed": false,
                "skipped_no_diff_expected": true,
                "task_id": task.id,
            }));
        }
        return Err(outside_worktree_or_empty_diff_error(
            &task.id,
            &workspace_path,
        ));
    }

    let message = batch_commit_message(task);
    let resolved_model = host.resolved_crew_model(batch_id)?;

    git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;
    let commit_sha = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
    commit_shas.push(commit_sha.trim().to_string());
    let mut result = json!({
        "phase": "commit",
        "decision": "performed",
        "committed": true,
        "adopted_commits": commit_shas.len() > 1,
        "commit_sha": commit_sha.trim(),
        "commit_shas": commit_shas,
        "job_run_id": batch_id,
        "skipped_no_diff_expected": false,
        "task_id": task.id,
    });
    if let Some(base_sha) = base_sha {
        result["base_sha"] = json!(base_sha);
    }
    Ok(result)
}

/// Commit a terminally-failed shipment's dirty candidate without consulting
/// the normal success-summary delivery gate. ADR-0246 confines this bypass to
/// the failure handoff, which blocks rather than promotes the task.
pub(super) fn commit_failure_candidate<H: RuntimeHost + ?Sized>(
    host: &H,
    run_id: &str,
    workspace_path: &Path,
    task: &orbit_common::types::Task,
) -> Result<(String, Vec<String>), OrbitError> {
    ensure_named_branch(workspace_path)?;
    ensure_no_unmerged_changes(workspace_path)?;
    git_success(workspace_path, &["add", "--all", "--", "."])?;
    let changed_files = staged_changed_files(workspace_path)?;
    if !changed_files.is_empty() {
        let message = batch_commit_message(task);
        let resolved_model = host.resolved_crew_model(run_id)?;
        git_commit_with_identity(workspace_path, &message, resolved_model.as_deref())?;
    }
    let head_sha = git_output(workspace_path, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    Ok((head_sha.trim().to_string(), changed_files))
}

fn existing_batch_commits(
    workspace_path: &Path,
    input: &Value,
    task_id: &str,
) -> Result<(Option<String>, Vec<String>), OrbitError> {
    let Some(base_ref) = input_string_field(input, "base_ref") else {
        return Ok((None, Vec::new()));
    };

    let base_commit = format!("{base_ref}^{{commit}}");
    let base_sha = git_output(workspace_path, &["rev-parse", "--verify", &base_commit])?;
    if !git_command_success(
        workspace_path,
        &["merge-base", "--is-ancestor", &base_sha, "HEAD"],
    )? {
        return Err(outside_worktree_or_empty_diff_error(
            task_id,
            workspace_path,
        ));
    }

    let range = format!("{base_sha}..HEAD");
    let commit_shas = git_output(workspace_path, &["rev-list", "--reverse", &range])?
        .lines()
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    Ok((Some(base_sha), commit_shas))
}

fn outside_worktree_or_empty_diff_error(task_id: &str, workspace_path: &Path) -> OrbitError {
    OrbitError::Execution(format!(
        "commit_batch_changes: no staged changes to commit for task '{task_id}' in worktree '{}'; \
         the implement step produced an empty diff. Changes may have been written outside \
         the assigned worktree, or attribution may be unknown; Orbit did not inspect, stage, \
         reset, or reconcile any other checkout",
        workspace_path.display()
    ))
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
