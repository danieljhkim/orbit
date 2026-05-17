use std::path::Path;

use orbit_common::types::{ExternalRef, OrbitError, ReviewThreadStatus, Role, Task, TaskStatus};
use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::super::super::input::{
    canonicalize_existing_dir, input_string_field, required_job_run_id,
};
use super::super::freshness::ensure_branch_fresh_against_base;
use super::super::git::{base_sync_mode_from_input, git_command_success, git_output};
use super::attribution::ship_done_attribution;

pub(in crate::executor::automation) fn git_merge<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "git_merge")?;
    if host
        .list_tasks_filtered(None, None, None, Some(batch_id), None, None)?
        .is_empty()
    {
        return Ok(json!({}));
    }

    let strategy = input
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("fast_forward");
    match strategy {
        "fast_forward" => super::super::worktree::merge_batch_worktree_into_base(host, input),
        "pr_merge" => merge_batch_pr(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "git_merge: unknown strategy '{other}'; expected fast_forward or pr_merge"
        ))),
    }
}

pub(super) fn merge_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "merge_batch_pr")?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    if batch_tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "merge_batch_pr: no tasks found for job_run_id '{batch_id}'"
        )));
    }

    // Find the GitHub PR external ref from the first task that has one.
    let pr_number = batch_tasks
        .iter()
        .find_map(Task::github_pr_number)
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "merge_batch_pr: no task in batch has a github-pr external ref".to_string(),
            )
        })?
        .to_string();

    let workspace_path = resolve_batch_workspace_path(host, input, batch_id)?;

    // Get the current branch from the workspace
    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;

    // Check that ALL tasks have APPROVED pr_status
    for task in &batch_tasks {
        let pr_status_raw = task.pr_status.as_deref().unwrap_or("none");
        let review_decision = super::super::super::review::normalize_review_decision(pr_status_raw);
        if review_decision != "APPROVED" {
            return Err(OrbitError::Execution(format!(
                "task '{}' is not approved (pr_status={pr_status_raw})",
                task.id
            )));
        }
    }

    // Check that ALL tasks are in Review or Done status
    for task in &batch_tasks {
        if !matches!(task.status, TaskStatus::Review | TaskStatus::Done) {
            return Err(OrbitError::Execution(format!(
                "task '{}' must be in Review or Done before merge_batch_pr; current status is {}",
                task.id, task.status
            )));
        }
    }

    ensure_branch_fresh_against_base(&workspace_path, &head, &base, base_sync_mode)?;

    let tool_context = ToolContext {
        cwd: Some(workspace_path.to_string_lossy().to_string()),
        allowed_tools: vec![],
        ..Default::default()
    };
    host.run_tool_with_context_and_role(
        "github.pr.merge",
        json!({
            "pr": pr_number,
            "strategy": "squash",
            // Do not pass --delete-branch to `gh pr merge` because the local
            // branch is still attached to the shared worktree and `gh` would
            // fail trying to delete it.  We delete the remote branch separately
            // below, tolerating errors (the repo may auto-delete branches after
            // merge).
            "delete_branch": false,
        }),
        Role::Admin,
        tool_context,
    )?;

    // Best-effort remote branch cleanup.  Some repos have GitHub's
    // "Automatically delete head branches" enabled, so the remote ref may
    // already be gone — ignore errors.
    let _ = git_command_success(&workspace_path, &["push", "origin", "--delete", &head]);

    let batch_requires_revision = batch_tasks
        .iter()
        .map(|task| task_required_revision(host, task))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .any(|requires_revision| requires_revision);
    let batch_author = batch_tasks.iter().find_map(ship_done_attribution);

    // Preserve ship attribution per task across the Review -> Done transition.
    // See `merge_batch_pr_preserves_task_attribution_per_task`: the source of
    // truth is task.implemented_by -> task.created_by -> system fallback.
    for task in &batch_tasks {
        host.apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                status: if task.status == TaskStatus::Review {
                    Some(TaskStatus::Done)
                } else {
                    None
                },
                external_refs: vec![ExternalRef::github_pr(pr_number.clone())?],
                model: ship_done_attribution(task),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    if host.scoring_enabled()
        && let Some(model) = batch_author
    {
        let _ = if batch_requires_revision {
            pr_scoreboard::record_pr_count_with_revision(host.scoreboard_dir(), &model)
        } else {
            pr_scoreboard::record_pr_count_without_revision(host.scoreboard_dir(), &model)
        };
    }

    Ok(json!({ "merged": true }))
}

fn resolve_batch_workspace_path<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
    batch_id: &str,
) -> Result<std::path::PathBuf, OrbitError> {
    match input_string_field(input, "workspace_path") {
        Some(path) => canonicalize_existing_dir(&path, "workspace_path"),
        None => {
            let repo_root = host.repo_root()?;
            super::super::worktree::resolve_shared_worktree_path(Path::new(&repo_root), batch_id)
        }
    }
}

fn task_required_revision<H: TaskHost + ?Sized>(host: &H, task: &Task) -> Result<bool, OrbitError> {
    let history = host.get_task_history(&task.id)?;
    let review_threads = host.get_task_review_threads(&task.id)?;
    Ok(history.iter().any(|entry| {
        entry.event == "status_changed"
            && entry.from_status == Some(TaskStatus::Review)
            && matches!(
                entry.to_status,
                Some(TaskStatus::Backlog | TaskStatus::InProgress | TaskStatus::Rejected)
            )
    }) || review_threads
        .iter()
        .any(|thread| thread.status == ReviewThreadStatus::Resolved))
}
