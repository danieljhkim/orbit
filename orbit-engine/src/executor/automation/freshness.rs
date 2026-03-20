use std::path::Path;

use orbit_types::OrbitError;

use super::git::{fetch_remote_base, git_output, resolve_worktree_start_point};

#[derive(Debug, Clone)]
pub(super) struct BranchFreshness {
    pub(super) base_ref: String,
    pub(super) head_ref: String,
    pub(super) commits_behind: u64,
    pub(super) commits_ahead: u64,
}

pub(super) fn ensure_branch_fresh_against_base(
    repo_root: &Path,
    head: &str,
    base: &str,
) -> Result<BranchFreshness, OrbitError> {
    fetch_remote_base(repo_root, base);
    let base_ref = resolve_worktree_start_point(repo_root, base)?;
    let divergence = git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...{head}"),
        ],
    )?;
    let mut parts = divergence.split_whitespace();
    let commits_behind = parse_divergence_count(parts.next(), "behind", base, head)?;
    let commits_ahead = parse_divergence_count(parts.next(), "ahead", base, head)?;
    if parts.next().is_some() {
        return Err(OrbitError::Execution(format!(
            "unexpected git divergence output while comparing '{head}' to '{base_ref}': {divergence}"
        )));
    }

    if commits_behind > 0 {
        return Err(OrbitError::Execution(format!(
            "task branch '{head}' is behind base '{base_ref}' by {commits_behind} commit(s); refresh the task branch before opening or merging the PR"
        )));
    }

    Ok(BranchFreshness {
        base_ref,
        head_ref: head.to_string(),
        commits_behind,
        commits_ahead,
    })
}

fn parse_divergence_count(
    value: Option<&str>,
    label: &str,
    base: &str,
    head: &str,
) -> Result<u64, OrbitError> {
    let raw = value.ok_or_else(|| {
        OrbitError::Execution(format!(
            "missing {label} divergence count while comparing '{head}' to '{base}'"
        ))
    })?;
    raw.parse::<u64>().map_err(|error| {
        OrbitError::Execution(format!(
            "invalid {label} divergence count '{raw}' while comparing '{head}' to '{base}': {error}"
        ))
    })
}
