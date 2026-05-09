use std::path::Path;

use orbit_common::types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;
use crate::executor::automation::input::{canonicalize_existing_dir, input_string_field};

use super::super::git::{
    BaseSyncMode, base_sync_mode_from_input, git_command_success, git_output, git_success,
    resolve_worktree_start_point,
};
use super::resolve_shared_worktree_path;

const DEFAULT_BASE: &str = "main";
const MAX_REBASE_RETRY_ATTEMPTS: usize = 2;

pub(in crate::executor::automation) fn merge_batch_worktree_into_base<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = crate::executor::automation::batch::require_run_id(
        input,
        "merge_batch_worktree_into_base",
    )?;
    let repo_root_str = host.repo_root()?;
    let repo_root = canonicalize_existing_dir(&repo_root_str, "repo_root")?;
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(path) => canonicalize_existing_dir(&path, "workspace_path")?,
        None => resolve_shared_worktree_path(&repo_root, run_id)?,
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

fn checkout_base_branch(
    repo_root: &Path,
    base: &str,
    start_point: &str,
    base_sync_mode: BaseSyncMode,
) -> Result<(), OrbitError> {
    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )? {
        git_success(repo_root, &["checkout", base])?;
        if base_sync_mode == BaseSyncMode::Remote {
            fast_forward_local_base_to_remote(repo_root, base, start_point)?;
        }
        return Ok(());
    }

    git_success(repo_root, &["checkout", "-B", base, start_point])?;
    Ok(())
}

fn merge_with_rebase_retry(
    repo_root: &Path,
    workspace_path: &Path,
    base: &str,
    workspace_branch: &str,
    base_sync_mode: BaseSyncMode,
) -> Result<(), OrbitError> {
    for attempt in 0..=MAX_REBASE_RETRY_ATTEMPTS {
        let start_point = resolve_worktree_start_point(repo_root, base, base_sync_mode)?;
        checkout_base_branch(repo_root, base, &start_point, base_sync_mode)?;
        if git_command_success(repo_root, &["merge", "--ff-only", workspace_branch])? {
            return Ok(());
        }
        if attempt == MAX_REBASE_RETRY_ATTEMPTS {
            return Err(OrbitError::Execution(format!(
                "merge_batch_worktree_into_base: failed to fast-forward merge '{workspace_branch}' into '{base}' after {} rebase retry attempts",
                MAX_REBASE_RETRY_ATTEMPTS
            )));
        }

        let updated_base = resolve_worktree_start_point(repo_root, base, base_sync_mode)?;
        if let Err(error) = git_success(workspace_path, &["rebase", &updated_base]) {
            let _ = git_success(workspace_path, &["rebase", "--abort"]);
            return Err(error);
        }
        ensure_clean_checkout(workspace_path, "shared batch worktree")?;
    }

    Ok(())
}

fn fast_forward_local_base_to_remote(
    repo_root: &Path,
    base: &str,
    remote_base: &str,
) -> Result<(), OrbitError> {
    let divergence = git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{remote_base}...{base}"),
        ],
    )?;
    let mut parts = divergence.split_whitespace();
    let remote_only = parse_divergence_count(parts.next(), "remote-only", remote_base, base)?;
    let local_only = parse_divergence_count(parts.next(), "local-only", remote_base, base)?;
    if parts.next().is_some() {
        return Err(OrbitError::Execution(format!(
            "unexpected git divergence output while comparing '{base}' to '{remote_base}': {divergence}"
        )));
    }

    if local_only > 0 {
        return Err(OrbitError::Execution(format!(
            "local base branch '{base}' has {local_only} commit(s) not present in '{remote_base}'; reconcile it or run with base_sync=local"
        )));
    }

    if remote_only > 0 {
        git_success(repo_root, &["merge", "--ff-only", remote_base])?;
    }

    Ok(())
}

fn parse_divergence_count(
    value: Option<&str>,
    label: &str,
    left: &str,
    right: &str,
) -> Result<u64, OrbitError> {
    let raw = value.ok_or_else(|| {
        OrbitError::Execution(format!(
            "missing {label} divergence count while comparing '{left}' to '{right}'"
        ))
    })?;
    raw.parse::<u64>().map_err(|error| {
        OrbitError::Execution(format!(
            "invalid {label} divergence count '{raw}' while comparing '{left}' to '{right}': {error}"
        ))
    })
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
