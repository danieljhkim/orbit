use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::git::{git_output, git_output_paths, git_success};
use super::input::{canonicalize_existing_dir, required_input_string, task_commit_message};

pub(super) fn commit_task_changes<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let workspace_path = canonicalize_existing_dir(
        &task.workspace_path.as_deref().ok_or_else(|| {
            OrbitError::InvalidInput("commit_task_changes requires task.workspace_path".to_string())
        })?,
        "workspace_path",
    )?;
    let expected_branch = format!("orbit/{task_id}");
    let summary = task.execution_summary.clone();
    if summary.trim().is_empty() {
        return Err(OrbitError::Execution(format!(
            "task '{}' commit_task_changes requires a non-empty execution_summary on the task",
            task_id
        )));
    }

    let actual_branch = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if actual_branch.trim() != expected_branch {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' is on branch '{}' but '{}' was expected",
            workspace_path.display(),
            actual_branch.trim(),
            expected_branch
        )));
    }

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;
    let changed_files = git_output_paths(
        &workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )?;

    // Idempotent: if there are no uncommitted changes (e.g. the agent already
    // committed), return success instead of erroring.
    if changed_files.is_empty() {
        git_success(&workspace_path, &["reset", "HEAD"])?;
        return Ok(json!({}));
    }

    let message = task_commit_message(&task.task_type, &task.title, &task.id, &summary);
    git_success(&workspace_path, &["commit", "-m", &message])?;
    Ok(json!({}))
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
