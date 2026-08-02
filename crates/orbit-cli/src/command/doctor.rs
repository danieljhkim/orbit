use clap::Args;
use orbit_cmd::{DoctorCommands, WorkspaceDoctorResult, WorkspaceDoctorStatus};
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

    /// Remove lock files whose recorded holder process is dead before diagnosing the workspace.
    #[arg(long)]
    pub fix_stale_locks: bool,

    /// Release only task reservations whose owner/task state is conclusively inactive.
    #[arg(long)]
    pub fix_stale_task_locks: bool,

    /// Remove retired graph state from this worktree and the shared workspace.
    #[arg(long)]
    pub remove_graph: bool,

    /// Abandon learning/ADR id allocations whose pinned worktree is gone and whose body is
    /// unreadable, before diagnosing the workspace. The ids are never reissued.
    #[arg(long)]
    pub fix_orphaned_allocations: bool,
}

impl Execute for DoctorCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.fix_stale_locks {
            let removed = runtime.remove_stale_lock_files()?;
            if !self.json {
                println!("Removed {removed} stale lock file(s).");
            }
        }
        if self.fix_stale_task_locks {
            let released = runtime.clear_stale_task_reservations()?;
            if !self.json {
                println!("Released {released} stale task reservation(s).");
            }
        }
        if self.remove_graph {
            let removed = runtime.remove_retired_graph_state()?;
            if !self.json {
                println!("Removed {removed} retired graph location(s).");
            }
        }
        if self.fix_orphaned_allocations {
            let cleared = runtime.clear_orphaned_id_allocations()?;
            if !self.json {
                println!("Abandoned {cleared} orphaned id allocation(s).");
            }
        }
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
                    Cell::new(human_detail(row)),
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

pub(crate) fn human_detail(row: &WorkspaceDoctorResult) -> String {
    row.remediation.as_ref().map_or_else(
        || row.message.clone(),
        |remediation| format!("{}\nAction: {remediation}", row.message),
    )
}

pub(crate) fn doctor_row_json(row: &WorkspaceDoctorResult) -> Value {
    json!({
        "check": row.check_name,
        "status": match row.status {
            WorkspaceDoctorStatus::Ok => "ok",
            WorkspaceDoctorStatus::Warning => "warning",
            WorkspaceDoctorStatus::Error => "error",
            WorkspaceDoctorStatus::Skipped => "skipped",
        },
        "message": row.message,
        "remediation": row.remediation,
    })
}
