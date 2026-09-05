//! `orbit run` command implementation split across focused submodules.
//!
//! - `types` — `JobRunListParams` and `JobRunCancelResult` DTOs.
//! - `actions` — cancel/archive/delete flows and pipeline state marking.
//! - `query` — list/show/history entry points plus backend queries.
//! - `reconcile` — stale-run reconciliation, terminal timing repair, audit parsing.
//! - `owner` — process signalling, owner identity classification, liveness probes (Unix + shims).
//! - `conflict` — recording a terminal outcome that contradicts the one already persisted.
//! - `tests/*` — helpers and regression tests split by concern (actions, reconcile, owner, conflict).

mod actions;
mod conflict;
mod owner;
mod projection;
mod query;
mod reconcile;
mod types;

#[cfg(test)]
mod tests;

#[cfg(unix)]
pub(crate) use actions::CANCELLATION_WORKER_EXIT_AUDIT;
#[cfg(test)]
pub(crate) use conflict::TERMINAL_OUTCOME_CONFLICT_CODE;
pub(crate) use owner::{RunOwnerLiveness, run_owner_liveness};
pub use projection::{
    ActivityInvocationEvidence, job_run_to_json, job_run_to_json_with_activity_provenance,
};
pub use types::{JobRunCancelResult, JobRunListParams};
