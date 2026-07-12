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
//! - [`fingerprint`] — finding signatures for open-task dedupe.
//! - [`prompt`] — composition of the QA agent prompt.
//! - [`report`] — parsing the agent's structured findings report.
//! - [`worker`] — the loopback client for the worker invoke daemon.
//! - [`sweep`] — the pass itself, including run-ledger recording.

pub mod config;
pub mod fingerprint;
mod git;
pub mod prompt;
pub mod report;
pub mod state;
pub mod sweep;
pub mod worker;

#[cfg(test)]
mod tests;

pub use config::{DEFAULT_WORKER_BASE_URL, QaSweepConfig, QaWorkspaceConfig};
pub use fingerprint::{QA_SWEEP_TAG, finding_fingerprint, fingerprint_tag};
pub use report::{Finding, QaReport, Severity, parse_report};
pub use state::{QaSweepState, QaWorkspaceWatermark};
pub use sweep::{
    QA_SWEEP_JOB, QaFindingReport, QaSweepOptions, QaSweepOutcome, QaWorkspaceReport, run_qa_sweep,
    run_qa_sweep_at,
};
