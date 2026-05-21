mod cleanup;
mod merge;
mod setup;

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

use super::git::{git_output, git_success};

pub(in crate::executor::automation) use cleanup::cleanup_worktree;
pub(in crate::executor::automation) use merge::merge_batch_worktree_into_base;
pub(in crate::executor::automation) use setup::setup_worktree;

const SHARED_WORKTREE_NAME_PREFIX: &str = "parallel-batch";
const SHARED_WORKTREE_BRANCH_PREFIX: &str = "orbit/parallel-batch";

pub(in crate::executor::automation) fn sanitize_worktree_token(
    value: &str,
) -> Result<String, OrbitError> {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized
        .trim_matches(|c: char| c == '-' || c == '.')
        .to_string();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "run_id '{value}' sanitizes to an empty string"
        )));
    }
    Ok(trimmed)
}

pub(in crate::executor::automation) fn resolve_worktree_path_from_prefix(
    repo_root: &Path,
    prefix: &str,
    run_id: &str,
) -> Result<PathBuf, OrbitError> {
    let sanitized = sanitize_worktree_token(run_id)?;
    let dir_name = format!("{prefix}-{sanitized}");
    match worktree_root() {
        Some(root) => Ok(root.join(repo_name(repo_root)?).join(dir_name)),
        None => Ok(repo_root
            .join(".orbit")
            .join("state")
            .join("worktrees")
            .join(dir_name)),
    }
}

pub(in crate::executor::automation) fn resolve_shared_worktree_path(
    repo_root: &Path,
    run_id: &str,
) -> Result<PathBuf, OrbitError> {
    let dir_name = shared_worktree_dir_name(run_id)?;
    match worktree_root() {
        Some(root) => Ok(root.join(repo_name(repo_root)?).join(dir_name)),
        None => Ok(repo_root
            .join(".orbit")
            .join("state")
            .join("worktrees")
            .join(dir_name)),
    }
}

pub(in crate::executor::automation) fn ensure_shared_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    start_point: &str,
    run_id: &str,
) -> Result<(), OrbitError> {
    let worktree_branch = shared_worktree_branch_name(run_id)?;
    let worktree_branch = worktree_branch.as_str();

    if worktree_path.exists() {
        let target = git_output(repo_root, &["rev-parse", start_point])?;
        git_success(
            worktree_path,
            &["checkout", "-B", worktree_branch, target.trim()],
        )?;
        git_success(worktree_path, &["clean", "-fd"])?;
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create shared worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    git_success(
        repo_root,
        &[
            "worktree",
            "add",
            "-b",
            worktree_branch,
            &worktree_path.to_string_lossy(),
            start_point,
        ],
    )
}

fn worktree_root() -> Option<PathBuf> {
    std::env::var("ORBIT_WORKTREE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn repo_name(repo_root: &Path) -> Result<&str, OrbitError> {
    repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot derive repository name from '{}'",
                repo_root.display()
            ))
        })
}

fn shared_worktree_dir_name(run_id: &str) -> Result<String, OrbitError> {
    Ok(format!(
        "{SHARED_WORKTREE_NAME_PREFIX}-{}",
        sanitize_worktree_token(run_id)?
    ))
}

fn shared_worktree_branch_name(run_id: &str) -> Result<String, OrbitError> {
    // Use a dash separator so the branch does not nest under the legacy
    // `orbit/parallel-batch` ref name.
    Ok(format!(
        "{SHARED_WORKTREE_BRANCH_PREFIX}-{}",
        sanitize_worktree_token(run_id)?
    ))
}

#[cfg(test)]
mod tests;
