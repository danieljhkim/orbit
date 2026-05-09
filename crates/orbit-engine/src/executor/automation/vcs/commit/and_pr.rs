use std::path::Path;

use orbit_common::types::OrbitError;
use serde_json::Value;

use crate::context::{RuntimeHost, TaskHost};

/// Combined automation: commit batch changes, then open a PR.
///
/// Calls the existing `commit_batch_changes` and `open_batch_pr` sequentially,
/// merging their JSON outputs into a single response.
pub(super) fn commit_and_open_batch_pr<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = crate::executor::automation::batch::require_run_id(
        input,
        "commit_and_open_batch_pr",
    )?
    .to_string();
    let input = ensure_workspace_path(host, input, &run_id)?;

    let mut commit_result = super::commit_batch_changes(host, &input)?;
    let pr_result = super::super::pr::open_batch_pr(host, &input)?;

    // Merge pr_result fields into commit_result so the caller gets a union of both outputs.
    if let (Some(base), Some(overlay)) = (commit_result.as_object_mut(), pr_result.as_object()) {
        for (key, value) in overlay {
            base.insert(key.clone(), value.clone());
        }
    }

    Ok(commit_result)
}

/// If `workspace_path` is missing from input, resolve it from the repo root.
fn ensure_workspace_path<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
    run_id: &str,
) -> Result<Value, OrbitError> {
    if input
        .get("workspace_path")
        .and_then(Value::as_str)
        .is_some()
    {
        return Ok(input.clone());
    }

    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);
    let worktree = super::super::worktree::resolve_shared_worktree_path(repo_root, run_id)?;

    let mut patched = input.clone();
    if let Some(obj) = patched.as_object_mut() {
        obj.insert(
            "workspace_path".to_string(),
            Value::String(worktree.to_string_lossy().to_string()),
        );
    }
    Ok(patched)
}
