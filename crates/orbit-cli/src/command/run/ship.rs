//! `orbit run ship` CLI entrypoint.

use clap::{Args, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime, build_ship_input, find_workflow};
use serde_json::Value;

use crate::command::Execute;

use super::support::{dispatch_workflow, print_workflow_dispatch_results};

pub(super) const SHIP_WORKFLOW: &str = "ship";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ShipMode {
    Pr,
    Local,
}

impl ShipMode {
    pub(super) fn to_core(self) -> orbit_core::ShipMode {
        match self {
            ShipMode::Pr => orbit_core::ShipMode::Pr,
            ShipMode::Local => orbit_core::ShipMode::Local,
        }
    }
}

#[derive(Args)]
#[command(
    about = "Ship backlog or explicitly selected tasks through the gated task pipeline",
    override_usage = "orbit run ship [<TASK_ID>...] [OPTIONS]",
    after_help = "Examples:\n  orbit run ship\n  orbit run ship T123\n  orbit run ship T123 T456 --mode local\n  orbit run ship T123 --base main\n\nInspect submitted runs with `orbit run history -j task_auto_pipeline` and `orbit run show <RUN_ID>`."
)]
pub struct ShipCommand {
    /// Optional task IDs to seed explicit gated shipment. Omit for auto mode.
    #[arg(value_name = "TASK_ID", num_args = 0..)]
    pub task_ids: Vec<String>,
    /// Pipeline mode for selected or auto-discovered task bundles.
    #[arg(short = 'm', long, value_enum, default_value = "pr")]
    pub mode: ShipMode,
    /// Base branch for shipment. Defaults to
    /// `[workflow] base_branch` from `config.toml` (or `main` if unset).
    #[arg(short = 'b', long)]
    pub base: Option<String>,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for ShipCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_ship_run_plan(&self, runtime.workflow_base_branch())?;
        let runs = dispatch_workflow(runtime, plan.workflow_alias, &plan.input, false, false, 1)?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

#[derive(Args)]
#[command(
    about = "Deprecated alias for `orbit run ship --mode local`",
    override_usage = "orbit run ship-local [<TASK_ID>...] [OPTIONS]",
    after_help = "`orbit run ship-local` was replaced by `orbit run ship --mode local`."
)]
pub struct LegacyShipLocalCommand {
    /// Deprecated. Pass task IDs to `orbit run ship --mode local`.
    #[arg(value_name = "TASK_ID", num_args = 0..)]
    pub task_ids: Vec<String>,
    /// Deprecated. Use `orbit run ship --mode local --base <BRANCH>`.
    #[arg(short = 'b', long)]
    pub base: Option<String>,
    /// Deprecated.
    #[arg(long)]
    pub json: bool,
}

impl Execute for LegacyShipLocalCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let _ = self;
        Err(OrbitError::InvalidInput(
            "`orbit run ship-local` was replaced by `orbit run ship --mode local`".to_string(),
        ))
    }
}

#[derive(Debug)]
pub(crate) struct WorkflowRunPlan {
    pub workflow_alias: &'static str,
    pub input: Value,
}

pub(crate) fn build_ship_run_plan(
    args: &ShipCommand,
    config_base_branch: &str,
) -> Result<WorkflowRunPlan, OrbitError> {
    validate_task_selection(&args.task_ids)?;
    let workflow_alias = SHIP_WORKFLOW;
    ensure_workflow_exists(workflow_alias)?;
    let base = args.base.as_deref().unwrap_or(config_base_branch);
    Ok(WorkflowRunPlan {
        workflow_alias,
        input: build_ship_input(args.mode.to_core(), base, &args.task_ids)?,
    })
}

fn validate_task_selection(task_ids: &[String]) -> Result<(), OrbitError> {
    if let Some(legacy) = task_ids.first().and_then(|value| legacy_ship_form(value)) {
        return Err(OrbitError::InvalidInput(legacy.to_string()));
    }
    Ok(())
}

fn legacy_ship_form(value: &str) -> Option<&'static str> {
    match value {
        "local" => {
            Some("`orbit run ship local` was replaced by `orbit run ship --mode local <TASK_ID>`")
        }
        "pr" => Some("`orbit run ship pr` was replaced by `orbit run ship --mode pr <TASK_ID>`"),
        "auto" | "ship-auto" => Some(
            "`orbit run ship auto` was replaced by `orbit run ship` (auto mode runs when no task ids are supplied)",
        ),
        "list" | "show" => Some(
            "`orbit run ship list/show` was removed; use `orbit run history -j <JOB_ID>` and `orbit run show <RUN_ID>` for run inspection",
        ),
        _ => None,
    }
}

fn ensure_workflow_exists(workflow_alias: &'static str) -> Result<(), OrbitError> {
    find_workflow(workflow_alias)
        .map(|_| ())
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))
}
