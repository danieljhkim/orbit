mod cleanup;
mod gc;
mod merge;
mod setup;

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde_json::Value;

pub use gc::{WorktreeGcOptions, WorktreeGcResult, collect_worktrees};
pub(in crate::executor::automation) use merge::merge_batch_worktree_into_base;
pub(in crate::executor::automation) use setup::setup_worktree;

const SHARED_WORKTREE_NAME_PREFIX: &str = "parallel-batch";

/// Extract the `run_id` from an activity input value, returning a trimmed
/// non-empty string. Used by activities that need to resolve the shared
/// worktree for a run.
fn require_run_id<'a>(input: &'a Value, activity: &str) -> Result<&'a str, OrbitError> {
    input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OrbitError::InvalidInput(format!("{activity} requires input.run_id")))
}

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

pub fn resolve_worktree_path_from_prefix(
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

pub fn resolve_shared_worktree_path(repo_root: &Path, run_id: &str) -> Result<PathBuf, OrbitError> {
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

#[cfg(test)]
mod tests;
