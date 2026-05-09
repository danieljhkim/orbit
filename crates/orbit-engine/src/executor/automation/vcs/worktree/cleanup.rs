use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::{Map, Value, json};

use crate::context::RuntimeHost;
use crate::executor::automation::input::input_string_field;

use super::super::git::{git_output, git_output_raw, git_success};
use super::{resolve_shared_worktree_path, resolve_worktree_path_from_prefix};

const DEFAULT_BRANCH_PREFIX: &str = "orbit";

pub(in crate::executor::automation) fn cleanup_worktree<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = crate::executor::automation::batch::require_run_id(input, "cleanup_worktree")?;
    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);
    let workspace_path = resolve_workspace_path(repo_root, input, run_id)?;
    let workspace_path_str = workspace_path.to_string_lossy().to_string();
    let branch_name = detect_branch_name(repo_root, &workspace_path);

    if workspace_path.exists() {
        git_success(
            repo_root,
            &["worktree", "remove", "--force", workspace_path_str.as_str()],
        )?;
    }
    git_success(repo_root, &["worktree", "prune"])?;
    if let Some(branch_name) = branch_name.as_deref() {
        git_success(repo_root, &["branch", "-D", branch_name])?;
    }

    let mut output = Map::new();
    output.insert("cleaned_up".to_string(), json!(true));
    output.insert("workspace_path".to_string(), json!(workspace_path_str));
    if let Some(branch_name) = branch_name {
        output.insert("branch".to_string(), json!(branch_name));
    }
    Ok(Value::Object(output))
}

fn resolve_workspace_path(
    repo_root: &Path,
    input: &Value,
    run_id: &str,
) -> Result<PathBuf, OrbitError> {
    if let Some(workspace_path) = input_string_field(input, "workspace_path") {
        return Ok(absolute_workspace_path(repo_root, &workspace_path));
    }

    if let Some(branch_prefix) = input_string_field(input, "branch_prefix") {
        return resolve_worktree_path_from_prefix(repo_root, &branch_prefix, run_id);
    }

    if has_task_id(input) {
        return resolve_worktree_path_from_prefix(repo_root, DEFAULT_BRANCH_PREFIX, run_id);
    }

    resolve_shared_worktree_path(repo_root, run_id)
}

fn absolute_workspace_path(repo_root: &Path, workspace_path: &str) -> PathBuf {
    let workspace_path = PathBuf::from(workspace_path);
    if workspace_path.is_absolute() {
        workspace_path
    } else {
        repo_root.join(workspace_path)
    }
}

fn has_task_id(input: &Value) -> bool {
    input
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|task_id| !task_id.is_empty())
}

fn detect_branch_name(repo_root: &Path, workspace_path: &Path) -> Option<String> {
    if workspace_path.is_dir()
        && let Ok(branch_name) = git_output(workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])
    {
        let branch_name = branch_name.trim();
        if !branch_name.is_empty() && branch_name != "HEAD" {
            return Some(branch_name.to_string());
        }
    }

    let worktree_list = git_output_raw(repo_root, &["worktree", "list", "--porcelain"]).ok()?;
    branch_name_from_worktree_list(&worktree_list, workspace_path)
}

fn branch_name_from_worktree_list(worktree_list: &str, workspace_path: &Path) -> Option<String> {
    let target_path = workspace_path.to_string_lossy();
    let mut matching_block = false;

    for line in worktree_list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            matching_block = path == target_path;
            continue;
        }

        if !matching_block {
            continue;
        }

        if let Some(branch_name) = line.strip_prefix("branch refs/heads/") {
            return Some(branch_name.to_string());
        }

        if line.is_empty() {
            matching_block = false;
        }
    }

    None
}
