//! `orbit run` command implementation split across focused submodules.
//!
//! - `types` — `JobRunListParams` and `JobRunCancelResult` DTOs.
//! - `actions` — cancel/archive/delete flows and pipeline state marking.
//! - `query` — list/show/history entry points plus backend queries.
//! - `reconcile` — stale-run reconciliation, terminal timing repair, audit parsing.
//! - `owner` — process signalling, owner identity classification, liveness probes (Unix + shims).
//! - `tests/*` — helpers and regression tests split by concern (cancel, reconcile, owner identity).

pub(crate) use crate::OrbitRuntime;

mod actions;
mod owner;
mod query;
mod reconcile;
mod types;

#[cfg(test)]
mod tests;

pub use types::{JobRunCancelResult, JobRunListParams};
