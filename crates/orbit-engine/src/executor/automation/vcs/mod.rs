mod commit;
mod freshness;
pub(crate) mod git;
mod pr;
mod pull;
mod push;
mod worktree;

pub(super) use commit::git_commit;
pub(super) use pr::{git_merge, pr_open};
pub(super) use pull::pull_batch_changes;
pub(super) use push::push_batch_changes;
pub(super) use worktree::{cleanup_worktree, setup_worktree};
pub(in crate::executor::automation) use worktree::{
    ensure_shared_worktree, resolve_shared_worktree_path,
};
