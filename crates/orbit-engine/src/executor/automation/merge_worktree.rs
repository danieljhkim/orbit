use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::git::{
    base_sync_mode_from_input, git_command_success, git_output, git_success,
    refresh_local_base_branch, resolve_worktree_start_point,
};
use super::input::{canonicalize_existing_dir, input_string_field};

const DEFAULT_BASE: &str = "main";
const MAX_REBASE_RETRY_ATTEMPTS: usize = 2;

pub(super) fn merge_batch_worktree_into_base<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = super::parallel::require_run_id(input, "merge_batch_worktree_into_base")?;
    let repo_root_str = host.repo_root()?;
    let repo_root = canonicalize_existing_dir(&repo_root_str, "repo_root")?;
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(path) => canonicalize_existing_dir(&path, "workspace_path")?,
        None => super::parallel::resolve_shared_worktree_path(&repo_root, run_id)?,
    };

    ensure_clean_checkout(&workspace_path, "shared batch worktree")?;

    let workspace_branch = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let workspace_branch = workspace_branch.trim().to_string();
    if workspace_branch == "HEAD" {
        return Err(OrbitError::Execution(
            "merge_batch_worktree_into_base: shared worktree is in detached HEAD state".to_string(),
        ));
    }

    ensure_clean_checkout(&repo_root, "base branch checkout")?;

    let base = input_string_field(input, "base").unwrap_or_else(|| DEFAULT_BASE.to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;
    merge_with_rebase_retry(
        &repo_root,
        &workspace_path,
        &base,
        &workspace_branch,
        base_sync_mode,
    )?;

    Ok(json!({
        "base": base,
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "workspace_branch": workspace_branch,
    }))
}

fn checkout_base_branch(repo_root: &Path, base: &str) -> Result<(), OrbitError> {
    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )? {
        git_success(repo_root, &["checkout", base])?;
        return Ok(());
    }

    let start_point = resolve_worktree_start_point(repo_root, base)?;
    git_success(repo_root, &["checkout", "-B", base, &start_point])?;
    Ok(())
}

fn merge_with_rebase_retry(
    repo_root: &Path,
    workspace_path: &Path,
    base: &str,
    workspace_branch: &str,
    base_sync_mode: super::git::BaseSyncMode,
) -> Result<(), OrbitError> {
    for attempt in 0..=MAX_REBASE_RETRY_ATTEMPTS {
        refresh_local_base_branch(repo_root, base, base_sync_mode);
        checkout_base_branch(repo_root, base)?;
        if git_command_success(repo_root, &["merge", "--ff-only", workspace_branch])? {
            return Ok(());
        }
        if attempt == MAX_REBASE_RETRY_ATTEMPTS {
            return Err(OrbitError::Execution(format!(
                "merge_batch_worktree_into_base: failed to fast-forward merge '{workspace_branch}' into '{base}' after {} rebase retry attempts",
                MAX_REBASE_RETRY_ATTEMPTS
            )));
        }

        let updated_base = resolve_worktree_start_point(repo_root, base)?;
        if let Err(error) = git_success(workspace_path, &["rebase", &updated_base]) {
            let _ = git_success(workspace_path, &["rebase", "--abort"]);
            return Err(error);
        }
        ensure_clean_checkout(workspace_path, "shared batch worktree")?;
    }

    Ok(())
}

fn ensure_clean_checkout(path: &Path, label: &str) -> Result<(), OrbitError> {
    let status = git_output(path, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(());
    }

    let has_unmerged = status.lines().any(|line| {
        let bytes = line.as_bytes();
        if bytes.len() < 2 {
            return false;
        }
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D')
    });
    if has_unmerged {
        return Err(OrbitError::Execution(format!(
            "{label} '{}' has unresolved merge conflicts",
            path.display()
        )));
    }

    Err(OrbitError::Execution(format!(
        "{label} '{}' must be clean before merge_batch_worktree_into_base",
        path.display()
    )))
}
