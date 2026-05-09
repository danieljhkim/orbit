use std::path::Path;

use orbit_common::types::OrbitError;

use super::git::{BaseSyncMode, git_command_success, git_output, resolve_worktree_start_point};

#[derive(Debug, Clone)]
pub(super) struct BranchFreshness {
    pub(super) base_ref: String,
    pub(super) head_ref: String,
    pub(super) commits_behind: u64,
    pub(super) commits_ahead: u64,
}

#[derive(Debug, Clone)]
pub(super) struct RebaseOutcome {
    pub(super) freshness: BranchFreshness,
    pub(super) rebased: bool,
}

/// Ensure `head` is not behind `base`, attempting a rebase onto `base` when it is.
///
/// Fast path: if `ensure_branch_fresh_against_base` returns `Ok`, we return
/// the freshness unchanged with `rebased = false`.
///
/// Recovery path: if the freshness check fails, we recompute the divergence
/// directly (NOT by parsing the error string) to determine whether the failure
/// was caused by the branch being behind. If so, we attempt
/// `git rebase <base_ref>`; on success, we re-check freshness and return
/// `rebased = true`. On conflict, we run `git rebase --abort` best-effort and
/// return the original error so the caller sees the semantically correct
/// "behind by N" failure. If the recomputed divergence shows the branch is
/// NOT actually behind, we propagate the original error unchanged because it
/// means the freshness check failed for a different reason.
pub(super) fn ensure_branch_rebased_onto_base(
    repo_root: &Path,
    head: &str,
    base: &str,
    sync_mode: BaseSyncMode,
) -> Result<RebaseOutcome, OrbitError> {
    let original_error = match ensure_branch_fresh_against_base(repo_root, head, base, sync_mode) {
        Ok(freshness) => {
            return Ok(RebaseOutcome {
                freshness,
                rebased: false,
            });
        }
        Err(error) => error,
    };

    // Recompute divergence directly — do NOT parse the original error string.
    let base_ref = match resolve_worktree_start_point(repo_root, base, sync_mode) {
        Ok(value) => value,
        Err(_) => return Err(original_error),
    };
    let divergence = match git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...{head}"),
        ],
    ) {
        Ok(value) => value,
        Err(_) => return Err(original_error),
    };
    let commits_behind: u64 = divergence
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    if commits_behind == 0 {
        // Freshness check failed for a reason other than being behind base.
        return Err(original_error);
    }

    // Attempt the rebase. `git_command_success` returns Ok(false) on non-zero
    // exit rather than mapping it to an Err, which is exactly what we want so
    // we can distinguish "rebase had conflicts" from the freshness-check error.
    let rebase_ok = git_command_success(repo_root, &["rebase", &base_ref]).unwrap_or(false);

    if !rebase_ok {
        // Best-effort abort to restore a clean worktree. Ignore errors from
        // the abort itself — the goal is to leave no rebase in progress.
        let _ = git_command_success(repo_root, &["rebase", "--abort"]);
        return Err(original_error);
    }

    let freshness = ensure_branch_fresh_against_base(repo_root, head, base, sync_mode)?;
    Ok(RebaseOutcome {
        freshness,
        rebased: true,
    })
}

pub(super) fn ensure_branch_fresh_against_base(
    repo_root: &Path,
    head: &str,
    base: &str,
    sync_mode: BaseSyncMode,
) -> Result<BranchFreshness, OrbitError> {
    let base_ref = resolve_worktree_start_point(repo_root, base, sync_mode)?;
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
