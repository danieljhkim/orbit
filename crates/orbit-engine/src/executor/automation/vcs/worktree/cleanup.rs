use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::{Map, Value, json};

use crate::context::RuntimeHost;
use crate::executor::automation::input::input_string_field;

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

    let mut output = Map::new();
    // Destructive cleanup is intentionally deferred until the owning run has
    // a persisted terminal timestamp. OrbitRuntime then invokes the shared
    // worktree GC collector, which applies the same retention and safety
    // classifier as `orbit gc worktrees --apply`.
    output.insert("cleaned_up".to_string(), json!(false));
    output.insert("cleanup_deferred".to_string(), json!(true));
    output.insert("workspace_path".to_string(), json!(workspace_path_str));
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
