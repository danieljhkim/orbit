//! Trailing QA validation for direct-push workspaces [ORB-10039].
//!
//! `orbit run qa-sweep` — sibling of `ship-sweep` (design D4, multi-env
//! operations): direct pushes to `agent-main` stay fast; this sweep validates
//! them on a lag, files fingerprint-deduped orbit tasks for regressions, and
//! advances a per-workspace last-validated watermark on green passes.
//!
//! - [`config`] — the `[qa]` section of the **global** `~/.orbit/config.toml`.
//! - [`state`] — per-workspace watermarks + the single-flight pass lock,
//!   both under the global orbit dir.
//! - [`fingerprint`] — failure signatures for open-task dedupe.
//! - [`sweep`] — the pass itself, including run-ledger recording.

pub mod config;
pub mod fingerprint;
mod git;
pub mod state;
pub mod sweep;

#[cfg(test)]
mod tests;

pub use config::{QaCheck, QaSweepConfig, QaWorkspaceConfig};
pub use fingerprint::{QA_SWEEP_TAG, failure_fingerprint, fingerprint_tag};
pub use state::{QaSweepState, QaWorkspaceWatermark};
pub use sweep::{
    QA_SWEEP_JOB, QaCheckReport, QaSweepOptions, QaSweepOutcome, QaWorkspaceReport, run_qa_sweep,
    run_qa_sweep_at,
};
