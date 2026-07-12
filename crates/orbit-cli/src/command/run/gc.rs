//! `orbit run gc` — reclaim orphaned pipeline worktrees [ORB-10173].
//!
//! Reconciles `.orbit/state/worktrees/*` against the run table and removes any
//! worktree with no live run, under the configured retention policy (failed /
//! timeout / interrupted worktrees are kept for a debugging window; success /
//! cancelled reap immediately). Orphaned non-terminal run records are cancelled
//! before their worktree is reclaimed. A live run's worktree is never touched,
//! so this is safe to run while other runs are in flight.

use clap::Args;
use orbit_core::command::job::WorktreeGcOptions;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
#[command(
    about = "Reclaim orphaned pipeline worktrees under the retention policy",
    after_help = "Reconciles `.orbit/state/worktrees/*` against the run table: reclaims worktrees \
with no live run, cancels orphaned non-terminal run records, and keeps recent failed-run \
worktrees for debugging. Never removes a worktree owned by a live run, so it is safe to run \
while pipelines are in flight.\n\nExamples:\n  orbit run gc\n  orbit run gc --dry-run --json\n  \
orbit run gc --failed-retention-days 3"
)]
pub struct RunGcArgs {
    /// Report what would be reclaimed without removing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Override the retention window (days) for failed/timeout/interrupted run
    /// worktrees. Defaults to the workspace `[worktree] gc_failed_retention_days`.
    #[arg(long)]
    pub failed_retention_days: Option<i64>,

    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunGcArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let options = WorktreeGcOptions {
            dry_run: self.dry_run,
            failed_retention_days: self.failed_retention_days,
        };
        let outcome = runtime.gc_worktrees(&options)?;

        if self.json {
            let entries: Vec<_> = outcome
                .entries
                .iter()
                .map(|entry| {
                    json!({
                        "worktree": entry.worktree,
                        "run_id": entry.run_id,
                        "run_state": entry.run_state.map(|state| state.to_string()),
                        "action": entry.action.as_str(),
                        "reason": entry.reason,
                        "cancelled_orphan": entry.cancelled_orphan,
                    })
                })
                .collect();
            return crate::output::json::print_pretty(&json!({
                "dry_run": self.dry_run,
                "scanned": outcome.scanned,
                "reclaimed": outcome.reclaimed,
                "cancelled_orphans": outcome.cancelled_orphans,
                "entries": entries,
            }));
        }

        for entry in &outcome.entries {
            let run = entry.run_id.as_deref().unwrap_or("-");
            println!("{:<14} {run}  {}", entry.action.as_str(), entry.worktree);
            if !entry.reason.is_empty() {
                println!("               {}", entry.reason);
            }
        }
        let verb = if self.dry_run {
            "would reclaim"
        } else {
            "reclaimed"
        };
        println!(
            "{verb} {}/{} worktree(s); cancelled {} orphaned run record(s)",
            outcome.reclaimed, outcome.scanned, outcome.cancelled_orphans
        );
        Ok(())
    }
}
