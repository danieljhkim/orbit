use std::path::Path;

use chrono::Utc;
use orbit_common::types::{ExternalRef, OrbitError, TaskComment, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate};
use crate::executor::automation::input::{
    canonicalize_existing_dir, input_string_field, required_input_string,
};

use super::commit::commit_failure_candidate;
use super::freshness::commit_sha;
use super::git::{
    base_sync_mode_from_input, git_command_success, git_output, resolve_worktree_start_point,
};
use super::pr::open_or_reuse_unchecked;
use super::push::push_batch_changes_inner;

const CONFLICT_BLOCKED_EVENT: &str = "pr_conflict_blocked";
const FAILURE_HANDOFF_EVENT: &str = "pr_failure_handoff";

/// ADR-0246 terminal hook for `task_pr_pipeline`.
///
/// The original job error remains authoritative. This action only makes the
/// candidate recoverable: restore the pre-rebase branch, commit dirtiness,
/// push without rewriting unknown remote history, publish a blocked PR, and
/// leave the task in `blocked` rather than `review`.
pub(in crate::executor::automation) fn pr_failure_handoff<H: RuntimeHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let failed_step_id = required_input_string(input, "failed_step_id")?;
    let error_code = required_input_string(input, "error_code")?;
    let error_message = required_input_string(input, "error_message")?;
    let run_id = required_input_string(input, "run_id")?;
    let job_input = input
        .get("job_input")
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.job_input".to_string()))?;
    let task_ids = job_input
        .get("task_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "pr_failure_handoff: job_input.task_ids must be an array".to_string(),
            )
        })?;
    let [task_id] = task_ids.as_slice() else {
        return Err(OrbitError::InvalidInput(format!(
            "pr_failure_handoff expected exactly one task id, got {}",
            task_ids.len()
        )));
    };
    let task_id = task_id.as_str().ok_or_else(|| {
        OrbitError::InvalidInput(
            "pr_failure_handoff: task id must be a non-empty string".to_string(),
        )
    })?;
    let task = host.get_task(task_id)?;
    if task.job_run_id.as_deref() != Some(run_id) {
        return Err(OrbitError::Execution(format!(
            "pr_failure_handoff: task '{}' no longer belongs to run '{}'",
            task.id, run_id
        )));
    }

    let worktree = pipeline_step(input, "worktree")?;
    let workspace_path = canonicalize_existing_dir(
        required_input_string(worktree, "workspace_path")?,
        "pipeline.worktree.workspace_path",
    )?;

    let mut conflicting_paths = unmerged_paths(&workspace_path)?;
    let rebase_aborted = git_command_success(&workspace_path, &["rebase", "--abort"])?;
    if !conflicting_paths.is_empty() && !rebase_aborted {
        return Err(OrbitError::Execution(
            "pr_failure_handoff: conflicts exist but the in-progress rebase could not be aborted"
                .to_string(),
        ));
    }
    if conflicting_paths.is_empty() {
        conflicting_paths = conflicts_from_error(error_message);
    }

    let (head_sha, committed_files) =
        commit_failure_candidate(host, run_id, &workspace_path, &task)?;
    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    if head == "HEAD" {
        return Err(OrbitError::Execution(
            "pr_failure_handoff: recovery candidate is detached".to_string(),
        ));
    }

    let base = input_string_field(job_input, "base_branch").unwrap_or_else(|| "main".to_string());
    let target_base_sha = match prepared_base_sha(input) {
        Some(base_sha) => base_sha,
        None => {
            let sync_mode = base_sync_mode_from_input(job_input)?;
            let target_base_ref = resolve_worktree_start_point(&workspace_path, &base, sync_mode)?;
            commit_sha(&workspace_path, &target_base_ref)?
        }
    };
    let original_base_sha = original_base_sha(&workspace_path, &head_sha, &target_base_sha)?;

    if committed_files.is_empty() && head_sha == original_base_sha {
        return Ok(json!({
            "phase": "failure_handoff",
            "decision": "no_candidate",
            "failed_step_id": failed_step_id,
            "workspace_path": workspace_path,
        }));
    }

    let pushed = push_batch_changes_inner(
        host,
        &json!({
            "branch": head,
            "workspace_path": workspace_path,
        }),
        &workspace_path,
    )?;
    let title = input_string_field(job_input, "title")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("[BLOCKED] {}", task.title.trim()));
    let body = blocked_pr_body(
        &task.id,
        run_id,
        failed_step_id,
        error_code,
        error_message,
        &original_base_sha,
        &target_base_sha,
        &conflicting_paths,
    );
    let (pr_number, pr_url, pr_created) =
        open_or_reuse_unchecked(host, &workspace_path, &head, &base, &title, &body)?;

    let conflict_blocked = !conflicting_paths.is_empty();
    let event = if conflict_blocked {
        CONFLICT_BLOCKED_EVENT
    } else {
        FAILURE_HANDOFF_EVENT
    };
    let paths = if conflicting_paths.is_empty() {
        "none reported".to_string()
    } else {
        conflicting_paths.join(", ")
    };
    let note = format!(
        "failure handoff published PR #{pr_number}: run={run_id}, failed_step={failed_step_id}, \
         original_base={original_base_sha}, target_base={target_base_sha}, conflicts={paths}"
    );
    host.apply_task_automation_update(
        &task.id,
        TaskAutomationUpdate {
            status: Some(TaskStatus::Blocked),
            status_event: Some(event.to_string()),
            status_note: Some(note.clone()),
            external_refs: vec![ExternalRef::github_pr(pr_number.clone())?],
            append_comments: vec![TaskComment {
                at: Utc::now(),
                by: "system".to_string(),
                message: format!("{note}\n\n{body}"),
            }],
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "phase": "failure_handoff",
        "decision": if conflict_blocked { "blocked_conflict_pr" } else { "blocked_failure_pr" },
        "failed_step_id": failed_step_id,
        "branch": head,
        "head_sha": head_sha,
        "original_base_sha": original_base_sha,
        "target_base_sha": target_base_sha,
        "conflicting_paths": conflicting_paths,
        "committed_files": committed_files,
        "push": pushed,
        "pr_number": pr_number,
        "pr_url": pr_url,
        "pr_created": pr_created,
        "task_status": "blocked",
    }))
}

fn pipeline_step<'a>(input: &'a Value, step: &str) -> Result<&'a Value, OrbitError> {
    input
        .get("pipeline")
        .and_then(|pipeline| pipeline.get(step))
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "pr_failure_handoff: missing pipeline.{step} checkpoint"
            ))
        })
}

fn prepared_base_sha(input: &Value) -> Option<String> {
    input
        .get("pipeline")
        .and_then(|pipeline| pipeline.get("prepare_branch"))
        .and_then(|prepare| prepare.get("base_sha"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(ToOwned::to_owned)
}

fn unmerged_paths(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    Ok(
        git_output(workspace_path, &["diff", "--name-only", "--diff-filter=U"])?
            .lines()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn original_base_sha(
    workspace_path: &Path,
    head_sha: &str,
    target_base_sha: &str,
) -> Result<String, OrbitError> {
    match git_output(workspace_path, &["merge-base", head_sha, target_base_sha]) {
        Ok(sha) if !sha.trim().is_empty() => Ok(sha.trim().to_string()),
        _ => Ok(
            git_output(workspace_path, &["rev-parse", &format!("{head_sha}^")])?
                .trim()
                .to_string(),
        ),
    }
}

fn conflicts_from_error(error: &str) -> Vec<String> {
    let Some((_, paths)) = error.split_once("conflicting paths: ") else {
        return Vec::new();
    };
    paths
        .lines()
        .next()
        .unwrap_or_default()
        .split(", ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn blocked_pr_body(
    task_id: &str,
    run_id: &str,
    failed_step_id: &str,
    error_code: &str,
    error_message: &str,
    original_base_sha: &str,
    target_base_sha: &str,
    conflicting_paths: &[String],
) -> String {
    let conflicts = if conflicting_paths.is_empty() {
        "- None reported; inspect the failed pipeline step before merging.".to_string()
    } else {
        conflicting_paths
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## Manual resolution required\n\n\
         Orbit preserved and pushed this task's pre-rebase candidate after the shipment pipeline \
         stopped. This PR is intentionally blocked and must be reconciled manually before merge.\n\n\
         - Task: `{task_id}`\n\
         - Run: `{run_id}`\n\
         - Failed step: `{failed_step_id}`\n\
         - Error code: `{error_code}`\n\
         - Original base: `{original_base_sha}`\n\
         - Target base: `{target_base_sha}`\n\n\
         ## Conflicting paths\n\n{conflicts}\n\n\
         ## Failure\n\n```text\n{error_message}\n```"
    )
}
