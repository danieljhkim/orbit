mod commit;
mod failure;
mod freshness;
pub(crate) mod git;
mod handoff;
mod pr;
mod pull;
mod push;
mod worktree;

pub(super) use commit::git_commit;
pub(super) use failure::pr_failure_handoff;
pub(super) use freshness::{prepare_pr_handoff, rebase_pr_branch};
pub(super) use pr::{git_merge, pr_open, pr_promote};
pub(super) use pull::pull_batch_changes;
pub(super) use push::push_batch_changes;
pub use worktree::{
    WorktreeGcOptions, WorktreeGcReport, WorktreeGcResult, collect_worktrees,
    resolve_shared_worktree_path, resolve_worktree_path_from_prefix,
};
pub(super) use worktree::{cleanup_worktree, setup_worktree};

#[cfg(test)]
mod tests;
