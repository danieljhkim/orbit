//! `orbit sweep` — the stateless routine scheduler pass [ORB-10021].
//!
//! Invoked every minute by the OS clock (launchd / systemd; see
//! `orbit routine init --install-clock`). Like `orbit run ship-sweep`, it
//! resolves everything from the global registry, never bootstraps a
//! `.orbit/` in the caller's cwd, and exits non-zero only on infrastructure
//! errors — an unconfigured host logs one line and exits 0, because the OS
//! clock will invoke it forever.

use clap::Args;
use orbit_core::OrbitError;
use orbit_core::routines::{SweepOptions, SweepOutcome, run_sweep};
use serde_json::json;

#[derive(Args)]
#[command(
    name = "sweep",
    about = "Fire due routines on this host (the scheduler pass the OS clock invokes)",
    after_help = "Loads routine definitions from every registered workspace with\n\
                  `[routines] role = \"source\"` in its config.toml, filters them for this\n\
                  host, and dispatches due targets as normal runs. Intended for the OS\n\
                  clock (launchd / systemd timer), e.g.:\n  orbit sweep --json\n\n\
                  Inspect routines with `orbit routine list`; dispatched fires appear in\n\
                  `orbit run history`."
)]
pub struct SweepCommand {
    /// Report what would fire without recording or dispatching anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl SweepCommand {
    /// Runs without a pre-initialized runtime: the sweep resolves every
    /// workspace from the global registry (per-workspace runtimes are built
    /// inside orbit-core).
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let outcome = run_sweep(SweepOptions {
            dry_run: self.dry_run,
        })?;

        if self.json {
            crate::output::json::print_pretty(&outcome_json(&outcome, self.dry_run))?;
            return Ok(());
        }

        if outcome.lock_busy {
            println!("sweep: another pass holds the lock on this host; exiting");
            return Ok(());
        }
        if outcome.reports.is_empty() && outcome.load_errors.is_empty() {
            println!("sweep[{}]: no routines configured", outcome.host_id);
            return Ok(());
        }
        for report in &outcome.reports {
            let mut line = format!("{} ({}): {}", report.routine, report.source, report.action);
            if let Some(reason) = &report.reason {
                line.push_str(&format!(" — {reason}"));
            }
            if let Some(slot) = &report.slot {
                line.push_str(&format!(" — slot {slot}"));
            }
            if let Some(run_id) = &report.run_id {
                line.push_str(&format!(" — run {run_id}"));
            }
            println!("{line}");
        }
        for error in &outcome.load_errors {
            let path = error
                .path
                .as_ref()
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default();
            eprintln!(
                "load error [{}]{}: {}",
                error.source_workspace, path, error.message
            );
        }
        Ok(())
    }
}

fn outcome_json(outcome: &SweepOutcome, dry_run: bool) -> serde_json::Value {
    json!({
        "host_id": outcome.host_id,
        "dry_run": dry_run,
        "lock_busy": outcome.lock_busy,
        "fired": outcome
            .reports
            .iter()
            .filter(|r| r.action == "fired" || r.action == "retry_fired")
            .count(),
        "reports": outcome.reports.iter().map(|r| json!({
            "routine": r.routine,
            "source": r.source,
            "action": r.action,
            "reason": r.reason,
            "slot": r.slot,
            "run_id": r.run_id,
        })).collect::<Vec<_>>(),
        "load_errors": outcome.load_errors.iter().map(|e| json!({
            "source_workspace": e.source_workspace,
            "path": e.path.as_ref().map(|p| p.display().to_string()),
            "message": e.message,
        })).collect::<Vec<_>>(),
    })
}
