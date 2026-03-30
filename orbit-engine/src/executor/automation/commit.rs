use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::git::{git_output, git_output_paths, git_success};
use super::input::{canonicalize_existing_dir, required_input_string};

pub(super) fn commit_batch_changes<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let workspace_path_str = required_input_string(input, "workspace_path")?;
    let workspace_path = canonicalize_existing_dir(workspace_path_str, "workspace_path")?;

    let actual_branch = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let actual_branch = actual_branch.trim();
    if actual_branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has detached HEAD; expected a named branch",
            workspace_path.display(),
        )));
    }

    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("commit_batch_changes requires input.run_id".to_string())
        })?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    let completed_task_ids: Vec<String> = batch_tasks.iter().map(|t| t.id.clone()).collect();

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "commit_batch_changes: no tasks found for batch_id '{batch_id}'"
        )));
    }

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = git_output_paths(
        &workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )?;

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
