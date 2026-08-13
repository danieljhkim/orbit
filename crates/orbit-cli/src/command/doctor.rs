use clap::Args;
use orbit_cmd::{DoctorCommands, WorkspaceDoctorResult, WorkspaceDoctorStatus};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::{Block, CommandOut, Execute, Payload};
use crate::output::color::{Domain, Role};

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
}

impl Execute for DoctorCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
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
        let results = runtime.doctor_workspace()?;
        let failures = results
            .iter()
            .filter(|row| row.status == WorkspaceDoctorStatus::Error)
            .count();
        let warnings = results
            .iter()
            .filter(|row| row.status == WorkspaceDoctorStatus::Warning)
            .count();

        let values = results.iter().map(doctor_row_json).collect::<Vec<_>>();
        let mut blocks = Vec::new();
        {
            use crate::output::table::{Column, Table};
            let mut table = Table::new(vec![
                Column::new("CHECK").fixed(),
                Column::new("STATUS").fixed(),
                Column::new("DETAILS"),
            ])
            .empty_message("no workspace checks ran");
            for row in &results {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&row.check_name),
                    crate::output::color::cell(status_label(row.status), Domain::DoctorStatus),
                    Cell::new(human_detail(row)),
                ]);
            }
            blocks.push(Block::table(table));

            if failures == 0 && warnings == 0 {
                blocks.push(Block::text(format!(
                    "\n{}",
                    crate::output::color::text("Workspace healthy.", Role::Ok)
                )));
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
        Ok(Payload::blocks(Value::Array(values), blocks).into())
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
