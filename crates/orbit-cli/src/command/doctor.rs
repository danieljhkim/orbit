use clap::Args;
use orbit_core::command::doctor::{WorkspaceDoctorResult, WorkspaceDoctorStatus};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

/// `orbit doctor` — workspace-level self-diagnostics [ORB-10005].
#[derive(Args)]
#[command(about = "Diagnose workspace health (config, database, disk, indexes, locks, runs)")]
pub struct DoctorCommand {
    /// Emit machine-readable JSON instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Execute for DoctorCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let results = runtime.doctor_workspace()?;
        let failures = results
            .iter()
            .filter(|row| row.status == WorkspaceDoctorStatus::Error)
            .count();
        let warnings = results
            .iter()
            .filter(|row| row.status == WorkspaceDoctorStatus::Warning)
            .count();

        if self.json {
            let values = results.iter().map(doctor_row_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))?;
        } else {
            let mut table = crate::output::table::build_table(&["CHECK", "STATUS", "DETAILS"]);
            for row in &results {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&row.check_name),
                    crate::output::color::doctor_status_color_cell(status_label(row.status)),
                    Cell::new(&row.message),
                ]);
            }
            println!("{table}");

            if failures == 0 && warnings == 0 {
                println!(
                    "\n{}",
                    crate::output::color::job_state_color("Workspace healthy.")
                );
            } else {
                eprintln!("\n{failures} failure(s), {warnings} warning(s).");
            }
        }

        // Unlike `skill doctor` / `tool doctor`, a failed check exits nonzero
        // so unattended callers (cron, CI, systemd) can alert on it.
        if failures > 0 {
            return Err(OrbitError::Execution(format!(
                "{failures} doctor check(s) failed"
            )));
        }
        Ok(())
    }
}

fn status_label(status: WorkspaceDoctorStatus) -> &'static str {
    match status {
        WorkspaceDoctorStatus::Ok => "ok",
        WorkspaceDoctorStatus::Warning => "warning",
        WorkspaceDoctorStatus::Error => "ERROR",
        WorkspaceDoctorStatus::Skipped => "skipped",
    }
}

fn doctor_row_json(row: &WorkspaceDoctorResult) -> Value {
    json!({
        "check": row.check_name,
        "status": match row.status {
            WorkspaceDoctorStatus::Ok => "ok",
            WorkspaceDoctorStatus::Warning => "warning",
            WorkspaceDoctorStatus::Error => "error",
            WorkspaceDoctorStatus::Skipped => "skipped",
        },
        "message": row.message,
    })
}
