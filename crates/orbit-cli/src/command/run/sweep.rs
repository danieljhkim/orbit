//! `orbit run ship-sweep` — dispatch ship runs across every registered
//! workspace that opted in (`[workflow] auto_ship = true`) and has ready
//! backlog work. Designed for unattended schedulers (systemd timer, cron):
//! it never bootstraps a workspace from the caller's cwd, isolates
//! per-workspace failures, and exits non-zero only when a workspace errored.

use std::path::Path;

use clap::Args;
use orbit_common::types::{Workspace, WorkspaceStatus};
use orbit_core::workspace_registry;
use orbit_core::{
    JobRunState, OrbitError, OrbitRuntime, TaskStatus, build_task_status_index,
    task_dependencies_ready,
};
use serde_json::{Value, json};

use super::ship::ShipMode;
use super::support::TASK_AUTO_PIPELINE_JOB;

#[derive(Args)]
#[command(
    name = "ship-sweep",
    about = "Dispatch ship runs in every registered workspace with ready backlog tasks",
    after_help = "Only workspaces with `[workflow] auto_ship = true` in their config.toml are\n\
                  swept; everything else is reported as skipped. Intended to run from a\n\
                  scheduler (systemd timer / cron), e.g.:\n  orbit run ship-sweep --json\n\n\
                  Inspect dispatched runs per workspace with `orbit run history -j task_auto_pipeline`."
)]
pub struct ShipSweepCommand {
    /// Pipeline mode for dispatched ship runs.
    #[arg(short = 'm', long, value_enum, default_value = "pr")]
    pub mode: ShipMode,
    /// Report what would be dispatched without submitting any run.
    #[arg(long)]
    pub dry_run: bool,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

struct SweepReport {
    workspace_id: String,
    workspace_name: String,
    action: &'static str,
    reason: Option<String>,
    ready_backlog: usize,
    run_id: Option<String>,
    run_state: Option<&'static str>,
}

impl SweepReport {
    fn skipped(ws: &Workspace, reason: &str, ready_backlog: usize) -> Self {
        Self {
            workspace_id: ws.id.clone(),
            workspace_name: ws.name.clone(),
            action: "skipped",
            reason: Some(reason.to_string()),
            ready_backlog,
            run_id: None,
            run_state: None,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "workspace_id": self.workspace_id,
            "workspace_name": self.workspace_name,
            "action": self.action,
            "reason": self.reason,
            "ready_backlog": self.ready_backlog,
            "run_id": self.run_id,
            "run_state": self.run_state,
        })
    }

    fn to_line(&self) -> String {
        let mut line = format!(
            "{}: {} (ready backlog: {})",
            self.workspace_name, self.action, self.ready_backlog
        );
        if let Some(reason) = &self.reason {
            line.push_str(&format!(" — {reason}"));
        }
        if let Some(run_id) = &self.run_id {
            line.push_str(&format!(" — run {run_id}"));
            if let Some(state) = self.run_state {
                line.push_str(&format!(" [{state}]"));
            }
        }
        line
    }
}

impl ShipSweepCommand {
    /// Runs without a pre-initialized runtime: the sweep resolves every
    /// workspace from the global registry and must never bootstrap a
    /// `.orbit/` in the scheduler's working directory.
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let global_root = workspace_registry::global_orbit_dir()?;
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        workspace_registry::validate_workspaces(&mut registry);
        workspace_registry::save_registry_to(&registry, &registry_path)?;

        let mode = self.mode.to_core();
        let reports: Vec<SweepReport> = registry
            .workspaces
            .iter()
            .map(|ws| sweep_workspace(&global_root, ws, mode, self.dry_run))
            .collect();

        let failed = reports.iter().filter(|r| r.action == "error").count();
        if self.json {
            crate::output::json::print_pretty(&json!({
                "dry_run": self.dry_run,
                "workspaces": reports.len(),
                "dispatched": reports.iter().filter(|r| r.action == "dispatched").count(),
                "would_dispatch": reports.iter().filter(|r| r.action == "would_dispatch").count(),
                "skipped": reports.iter().filter(|r| r.action == "skipped").count(),
                "failed": failed,
                "reports": reports.iter().map(SweepReport::to_json).collect::<Vec<_>>(),
            }))?;
        } else if reports.is_empty() {
            println!("no workspaces registered");
        } else {
            for report in &reports {
                println!("{}", report.to_line());
            }
        }

        if failed > 0 {
            return Err(OrbitError::WorkspaceError(format!(
                "ship-sweep: {failed} of {} workspace(s) failed",
                reports.len()
            )));
        }
        Ok(())
    }
}

fn sweep_workspace(
    global_root: &Path,
    ws: &Workspace,
    mode: orbit_core::ShipMode,
    dry_run: bool,
) -> SweepReport {
    if ws.status != WorkspaceStatus::Active || !ws.orbit_dir.exists() {
        return SweepReport::skipped(ws, "workspace_inactive", 0);
    }
    sweep_active_workspace(global_root, ws, mode, dry_run).unwrap_or_else(|error| SweepReport {
        workspace_id: ws.id.clone(),
        workspace_name: ws.name.clone(),
        action: "error",
        reason: Some(error.to_string()),
        ready_backlog: 0,
        run_id: None,
        run_state: None,
    })
}

fn sweep_active_workspace(
    global_root: &Path,
    ws: &Workspace,
    mode: orbit_core::ShipMode,
    dry_run: bool,
) -> Result<SweepReport, OrbitError> {
    let runtime = OrbitRuntime::from_roots(global_root, &ws.orbit_dir)?;
    if !runtime.workflow_auto_ship() {
        return Ok(SweepReport::skipped(ws, "auto_ship_disabled", 0));
    }

    let tasks = runtime.list_tasks()?;
    let status_by_id = build_task_status_index(&tasks);
    let ready_backlog = tasks
        .iter()
        .filter(|task| {
            task.status == TaskStatus::Backlog && task_dependencies_ready(task, &status_by_id)
        })
        .count();
    if ready_backlog == 0 {
        return Ok(SweepReport::skipped(ws, "no_ready_backlog", 0));
    }

    let in_flight = runtime
        .job_history(TASK_AUTO_PIPELINE_JOB)?
        .iter()
        .any(|run| {
            matches!(
                run.state,
                JobRunState::Pending | JobRunState::Running | JobRunState::Retrying
            )
        });
    if in_flight {
        return Ok(SweepReport::skipped(ws, "ship_in_flight", ready_backlog));
    }

    if dry_run {
        return Ok(SweepReport {
            workspace_id: ws.id.clone(),
            workspace_name: ws.name.clone(),
            action: "would_dispatch",
            reason: None,
            ready_backlog,
            run_id: None,
            run_state: None,
        });
    }

    let invoke = runtime.submit_ship_run(mode, None, &[], Some("ship-sweep"))?;
    Ok(SweepReport {
        workspace_id: ws.id.clone(),
        workspace_name: ws.name.clone(),
        action: "dispatched",
        reason: None,
        ready_backlog,
        run_id: Some(invoke.run_id),
        run_state: Some(if invoke.queued { "queued" } else { "submitted" }),
    })
}
