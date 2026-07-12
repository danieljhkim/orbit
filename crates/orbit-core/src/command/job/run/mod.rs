//! `orbit run` command implementation split across focused submodules.
//!
//! - `types` — `JobRunListParams` and `JobRunCancelResult` DTOs.
//! - `actions` — cancel/archive/delete flows and pipeline state marking.
//! - `query` — list/show/history entry points plus backend queries.
//! - `reconcile` — stale-run reconciliation, terminal timing repair, audit parsing.
//! - `owner` — process signalling, owner identity classification, liveness probes (Unix + shims).
//! - `gc` — pipeline worktree garbage collection: reconcile `.orbit/state/worktrees/*`
//!   against the run table and reclaim non-live worktrees under a retention policy (ORB-10173).
//! - `tests/*` — helpers and regression tests split by concern (actions, reconcile, owner).

mod actions;
mod gc;
mod owner;
mod query;
mod reconcile;
mod types;

#[cfg(test)]
mod tests;

pub use gc::{
    DEFAULT_FAILED_RETENTION_DAYS, WorktreeGcAction, WorktreeGcEntry, WorktreeGcOptions,
    WorktreeGcOutcome,
};
pub use types::{JobRunCancelResult, JobRunListParams};
