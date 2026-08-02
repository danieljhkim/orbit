use clap::Args;
use comfy_table::Cell;
use orbit_core::OrbitError;
use orbit_remote::routines::routine_statuses;
use orbit_remote::workspace_registry;
use serde_json::json;

use crate::output::table::{Column, Table};

#[derive(Args)]
pub struct RoutineListArgs {
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl RoutineListArgs {
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let global_root = workspace_registry::global_orbit_dir()?;
        let report = routine_statuses(&global_root)?;

        if self.json {
            let statuses: Vec<_> = report
                .statuses
                .iter()
                .map(|status| {
                    json!({
                        "name": status.routine.definition.name,
                        "source": status.routine.source_workspace,
                        "origin": status.routine.origin.as_str(),
                        "target": status.routine.definition.target.as_ref_string(),
                        "enabled": status.routine.definition.enabled,
                        "hosts": status.routine.definition.hosts,
                        "pinned_to_host": status.pinned_to_host,
                        "validation": &status.validation,
                        "paused_at": status.paused_at,
                        "effective": status.effective(),
                        "cron": status.routine.definition.trigger.cron,
                        "next_due": status.next_due,
                        "last_fire": status.last_fire.as_ref().map(|fire| json!({
                            "slot": fire.slot,
                            "attempt": fire.attempt,
                            "state": fire.state.as_str(),
                            "run_id": fire.run_id,
                        })),
                    })
                })
                .collect();
            crate::output::json::print_pretty(&json!({
                "host_id": report.host_id,
                "machine_id": report.machine_id,
                "registry": &report.registry,
                "routines": statuses,
                "load_errors": report.load_errors.iter().map(|e| json!({
                    "source_workspace": e.source_workspace,
                    "path": e.path.as_ref().map(|p| p.display().to_string()),
                    "message": e.message,
                })).collect::<Vec<_>>(),
            }))?;
            return Ok(());
        }

        if report.statuses.is_empty() && report.load_errors.is_empty() {
            eprintln!(
                "no routines found (host {}); mark a workspace with [routines] role = \"source\"",
                report.host_id
            );
            return Ok(());
        }

        // `orbit routine show <name>` prints a routine's full definition.
        let mut table = Table::new(vec![
            Column::new("NAME").fixed(),
            Column::new("SOURCE"),
            Column::new("ORIGIN").fixed(),
            Column::new("ENABLED").fixed(),
            Column::new("PINNED").fixed(),
            Column::new("PAUSED").fixed(),
            Column::new("NEXT DUE").fixed(),
            Column::new("LAST FIRE").fixed(),
        ])
        .empty_message(format!("no routines found (host {})", report.host_id));
        for status in &report.statuses {
            let last_fire = status
                .last_fire
                .as_ref()
                .map(|fire| format!("{} @ {}", fire.state.as_str(), fire.slot))
                .unwrap_or_else(|| "—".to_string());
            table.add_row(vec![
                Cell::new(&status.routine.definition.name),
                Cell::new(&status.routine.source_workspace),
                Cell::new(status.routine.origin.as_str()),
                Cell::new(if status.routine.definition.enabled {
                    "yes"
                } else {
                    "no"
                }),
                Cell::new(if status.pinned_to_host { "yes" } else { "no" }),
                Cell::new(if status.paused_at.is_some() {
                    "yes"
                } else {
                    "no"
                }),
                Cell::new(status.next_due.as_deref().unwrap_or("—")),
                Cell::new(last_fire),
            ]);
        }
        println!("host: {}", report.host_id);
        println!(
            "registry: {}/{}{}",
            report.registry.source,
            report.registry.state,
            report
                .registry
                .age_seconds
                .map(|age| format!(" ({age}s old)"))
                .unwrap_or_default()
        );
        table.print();
        for status in &report.statuses {
            for diagnostic in &status.validation.diagnostics {
                eprintln!(
                    "{} [{}:{}]: {}",
                    status.routine.definition.name,
                    diagnostic.severity.as_str(),
                    diagnostic.code,
                    diagnostic.message
                );
            }
        }
        for error in &report.load_errors {
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
