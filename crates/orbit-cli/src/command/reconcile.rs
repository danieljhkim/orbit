use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Reconcile pending/running job runs (detect stale, dispatch next steps)")]
pub struct ReconcileCommand {
    /// Run continuously with a 5-second interval
    #[arg(long)]
    pub watch: bool,

    /// List what would happen without dispatching
    #[arg(long)]
    pub dry_run: bool,
}

impl Execute for ReconcileCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.watch {
            loop {
                match runtime.reconcile_once(self.dry_run) {
                    Ok(outcome) => print_outcome(&outcome, self.dry_run),
                    Err(error) => eprintln!("reconcile: {error}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        } else {
            let outcome = runtime.reconcile_once(self.dry_run)?;
            print_outcome(&outcome, self.dry_run);
            Ok(())
        }
    }
}

fn print_outcome(outcome: &orbit_core::ReconcileOutcome, dry_run: bool) {
    let prefix = if dry_run { "[dry-run] " } else { "" };
    println!(
        "{}reconcile: processed={}, dispatched={}, completed={}, failed={}, errors={}",
        prefix,
        outcome.runs_processed,
        outcome.steps_dispatched,
        outcome.runs_completed,
        outcome.runs_failed,
        outcome.errors.len(),
    );
    for err in &outcome.errors {
        eprintln!("  error: {err}");
    }
}
