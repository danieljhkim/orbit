use clap::Args;
use orbit_core::OrbitError;
use orbit_core::routines::{recent_fires, routine_statuses};
use orbit_core::workspace_registry;
use serde_json::json;

const RECENT_FIRE_LIMIT: usize = 10;

#[derive(Args)]
pub struct RoutineShowArgs {
    /// Routine name.
    pub name: String,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl RoutineShowArgs {
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let global_root = workspace_registry::global_orbit_dir()?;
        let report = routine_statuses(&global_root)?;
        let Some(status) = report
            .statuses
            .iter()
            .find(|status| status.routine.definition.name == self.name)
        else {
            return Err(OrbitError::InvalidInput(format!(
                "no routine named '{}' (see `orbit routine list`)",
                self.name
            )));
        };
        let fires = recent_fires(&global_root, &self.name, RECENT_FIRE_LIMIT)?;
        let definition = &status.routine.definition;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "host_id": report.host_id,
                "machine_id": report.machine_id,
                "registry": &report.registry,
                "name": definition.name,
                "description": definition.description,
                "source": status.routine.source_workspace,
                "origin": status.routine.origin.as_str(),
                "path": status.routine.path.display().to_string(),
                "enabled": definition.enabled,
                "hosts": definition.hosts,
                "pinned_to_host": status.pinned_to_host,
                "validation": &status.validation,
                "paused_at": status.paused_at,
                "effective": status.effective(),
                "cron": definition.trigger.cron,
                "missed_run": definition.trigger.missed_run,
                "target": definition.target.as_ref_string(),
                "policy": definition.policy,
                "next_due": status.next_due,
                "recent_fires": fires.iter().map(|fire| json!({
                    "slot": fire.slot,
                    "attempt": fire.attempt,
                    "state": fire.state.as_str(),
                    "run_id": fire.run_id,
                    "detail": fire.detail,
                    "updated_at": fire.updated_at,
                })).collect::<Vec<_>>(),
            }))?;
            return Ok(());
        }

        println!("Name: {}", definition.name);
        if !definition.description.is_empty() {
            println!("Description: {}", definition.description);
        }
        println!(
            "Source: {} ({}, {} origin)",
            status.routine.source_workspace,
            status.routine.path.display(),
            status.routine.origin.as_str()
        );
        println!("Target: {}", definition.target.as_ref_string());
        println!(
            "Trigger: cron \"{}\" (missed_run: {})",
            definition.trigger.cron,
            match definition.trigger.missed_run {
                orbit_core::MissedRunPolicy::CatchUpOnce => "catch_up_once",
                orbit_core::MissedRunPolicy::Skip => "skip",
            }
        );
        println!(
            "Policy: timeout {}m, retries max {} (backoff {}m), overlap {}",
            definition.policy.timeout_minutes,
            definition.policy.retries.max,
            definition.policy.retries.backoff_minutes,
            match definition.policy.overlap {
                orbit_core::OverlapPolicy::Forbid => "forbid",
                orbit_core::OverlapPolicy::Allow => "allow",
            }
        );
        println!(
            "Enabled: {} | Pinned to {}: {} | Paused: {}",
            definition.enabled,
            report.host_id,
            status.pinned_to_host,
            status
                .paused_at
                .as_deref()
                .map(|at| format!("yes (since {at})"))
                .unwrap_or_else(|| "no".to_string())
        );
        println!(
            "Registry: {}/{}{}",
            report.registry.source,
            report.registry.state,
            report
                .registry
                .age_seconds
                .map(|age| format!(" ({age}s old)"))
                .unwrap_or_default()
        );
        for diagnostic in &status.validation.diagnostics {
            println!(
                "Validation [{}:{}]: {}",
                diagnostic.severity.as_str(),
                diagnostic.code,
                diagnostic.message
            );
        }
        println!(
            "Effective on this host: {}",
            if status.effective() { "yes" } else { "no" }
        );
        println!(
            "Next due: {}",
            status.next_due.as_deref().unwrap_or("unknown")
        );
        if fires.is_empty() {
            println!("Recent fires: none");
        } else {
            println!("Recent fires:");
            for fire in &fires {
                let run = fire
                    .run_id
                    .as_deref()
                    .map(|run_id| format!(" run {run_id}"))
                    .unwrap_or_default();
                let detail = fire
                    .detail
                    .as_deref()
                    .map(|detail| format!(" — {detail}"))
                    .unwrap_or_default();
                println!(
                    "  [{}] slot {} attempt {}{}{}",
                    fire.state.as_str(),
                    fire.slot,
                    fire.attempt,
                    run,
                    detail
                );
            }
        }
        Ok(())
    }
}
