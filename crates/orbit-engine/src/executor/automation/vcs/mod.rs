mod base_obsolescence;
mod commit;
mod delivery_marker;
mod failure;
mod freshness;
pub(crate) mod git;
mod handoff;
mod pr;
mod push;
mod worktree;

pub(super) use commit::git_commit;
pub(super) use failure::pr_failure_handoff;
pub(super) use freshness::{prepare_pr_handoff, rebase_pr_branch};
pub(super) use pr::{git_merge, pr_open, pr_promote};
pub(super) use push::push_batch_changes;
pub(super) use worktree::setup_worktree;
pub use worktree::{WorktreeGcOptions, WorktreeGcResult, collect_worktrees};

#[cfg(test)]
mod tests;
