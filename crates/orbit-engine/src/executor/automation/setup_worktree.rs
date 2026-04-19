use std::path::Path;

use orbit_types::{OrbitError, TaskStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::git::{
    base_sync_mode_from_input, git_success, refresh_local_base_branch,
    resolve_worktree_path_from_prefix, resolve_worktree_start_point,
};
use super::input::{input_string_field, required_input_string};

const DEFAULT_BASE: &str = "main";
const DEFAULT_BRANCH_PREFIX: &str = "orbit";

/// Create a worktree and branch for a single task or task bundle, stamp
/// `batch_id` and `workspace_path` on every task in scope, and move them to
/// `in_progress`.
///
/// Generic automation — not tied to duel or any specific workflow. Any
/// pipeline can reuse this by passing a `branch_prefix`.
pub(super) fn setup_worktree<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_ids = task_ids_from_input(input)?;
    let run_id = super::parallel::require_run_id(input, "setup_worktree")?;
    let base = input_string_field(input, "base").unwrap_or_else(|| DEFAULT_BASE.to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;
    let branch_prefix = input_string_field(input, "branch_prefix")
        .unwrap_or_else(|| DEFAULT_BRANCH_PREFIX.to_string());

    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);

    refresh_local_base_branch(repo_root, &base, base_sync_mode);

    let branch_name = branch_name_for_tasks(&branch_prefix, &task_ids);

    let worktree_path = resolve_worktree_path_from_prefix(repo_root, &branch_prefix, run_id)?;

    ensure_worktree(repo_root, &worktree_path, &base, &branch_name)?;

    let workspace_path_str = worktree_path.to_string_lossy().to_string();

    for task_id in &task_ids {
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                batch_id: Some(run_id.to_string()),
                workspace_path: Some(Some(workspace_path_str.clone())),
                status: Some(TaskStatus::InProgress),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

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
            "-B",
            branch_name,
            &worktree_path.to_string_lossy(),
            &start_point,
        ],
    )
}

fn task_ids_from_input(input: &Value) -> Result<Vec<String>, OrbitError> {
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        let task_ids = items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(
                            "setup_worktree input.task_ids entries must be non-empty strings"
                                .to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !task_ids.is_empty() {
            return Ok(task_ids);
        }
    }

    Ok(vec![required_input_string(input, "task_id")?.to_string()])
}

fn branch_name_for_tasks(branch_prefix: &str, task_ids: &[String]) -> String {
    if task_ids.len() == 1 {
        let short_ts = format!(
            "{:08x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        return format!("{branch_prefix}/{}-{short_ts}", task_ids[0]);
    }

    let mut sorted_ids = task_ids.to_vec();
    sorted_ids.sort();
    let digest = Sha256::digest(sorted_ids.join(","));
    let bundle_hash = format!("{digest:x}");
    format!("{branch_prefix}/bundle-{}", &bundle_hash[..8])
}
