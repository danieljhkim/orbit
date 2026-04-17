use std::path::Path;

use orbit_types::{OrbitError, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::git::{
    base_sync_mode_from_input, git_success, refresh_local_base_branch,
    resolve_worktree_path_from_prefix, resolve_worktree_start_point,
};
use super::input::{input_string_field, required_input_string};

const DEFAULT_BASE: &str = "main";
const DEFAULT_BRANCH_PREFIX: &str = "orbit";

/// Create a worktree and branch for a single task, stamp `batch_id` and
/// `workspace_path` on the task, and move it to `in_progress`.
///
/// Generic automation — not tied to duel or any specific workflow. Any
/// single-task pipeline can reuse this by passing a `branch_prefix`.
pub(super) fn setup_worktree<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let run_id = super::parallel::require_run_id(input, "setup_worktree")?;
    let base = input_string_field(input, "base").unwrap_or_else(|| DEFAULT_BASE.to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;
    let branch_prefix = input_string_field(input, "branch_prefix")
        .unwrap_or_else(|| DEFAULT_BRANCH_PREFIX.to_string());

    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);

    refresh_local_base_branch(repo_root, &base, base_sync_mode);

    let short_ts = format!(
        "{:08x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let branch_name = format!("{branch_prefix}/{task_id}-{short_ts}");

    let worktree_path = resolve_worktree_path_from_prefix(repo_root, &branch_prefix, run_id)?;

    ensure_worktree(repo_root, &worktree_path, &base, &branch_name)?;

    let workspace_path_str = worktree_path.to_string_lossy().to_string();

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            batch_id: Some(run_id.to_string()),
            workspace_path: Some(Some(workspace_path_str.clone())),
            status: Some(TaskStatus::InProgress),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "workspace_path": workspace_path_str,
        "head_ref": branch_name,
        "base_ref": base,
    }))
}

fn ensure_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    base: &str,
    branch_name: &str,
) -> Result<(), OrbitError> {
    if worktree_path.exists() {
        let target = super::git::git_output(repo_root, &["rev-parse", base])?;
        git_success(
            worktree_path,
            &["checkout", "-B", branch_name, target.trim()],
        )?;
        git_success(worktree_path, &["clean", "-fd"])?;
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    let start_point = resolve_worktree_start_point(repo_root, base)?;

    git_success(
        repo_root,
        &[
            "worktree",
            "add",
            "-b",
            branch_name,
            &worktree_path.to_string_lossy(),
            &start_point,
        ],
    )
}
