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
    after_help = "Examples:\n  orbit run ship\n  orbit run ship T123\n  orbit run ship T123 T456 --mode local\n  orbit run ship T123 --base main\n  orbit run ship T123 --review --review-crew opus-review\n\nInspect submitted runs with `orbit run history -j task_auto_pipeline` and `orbit run show <RUN_ID>`."
)]
pub struct ShipCommand {
    /// Optional task IDs to seed explicit gated shipment. Omit for auto mode.
    #[arg(value_name = "TASK_ID", num_args = 0..)]
    pub task_ids: Vec<String>,
    /// Pipeline mode for selected or auto-discovered task bundles. When omitted,
    /// the mode is resolved from the current workspace's registry entry
    /// (explicit `ship_mode`, else defaults to `local`).
    #[arg(short = 'm', long, value_enum)]
    pub mode: Option<ShipMode>,
    /// Base branch for shipment. Defaults to
    /// `[workflow] base_branch` from `config.toml` (or `main` if unset).
    #[arg(short = 'b', long)]
    pub base: Option<String>,
    /// Run an independent review after implementation and before shipment.
    /// Requires `--review-crew`.
    #[arg(long)]
    pub review: bool,
    /// Explicit crew used only for the independent review step.
    #[arg(long, value_name = "CREW")]
    pub review_crew: Option<String>,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for ShipCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mode = resolve_ship_mode(&self, runtime)?;
        let plan = build_ship_run_plan(&self, runtime.workflow_base_branch(), mode)?;
        let runs = dispatch_workflow(runtime, plan.workflow_alias, &plan.input, false, false, 1)?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

/// Resolve the effective ship mode for a `ship` invocation.
///
/// An explicit `--mode` wins. Otherwise the mode is resolved from the current
/// workspace's registry entry (matched by `orbit_dir`): explicit `ship_mode`,
/// else the `local` default. If the current workspace isn't found in the
/// registry, fall back to `local` (the safe default — a repo without an explicit
/// `pr` mode should never attempt a PR that could fail structurally).
fn resolve_ship_mode(
    args: &ShipCommand,
    runtime: &OrbitRuntime,
) -> Result<orbit_core::ShipMode, OrbitError> {
    if let Some(mode) = args.mode {
        return Ok(mode.to_core());
    }
    let registry = orbit_remote::workspace_registry::load_registry()?;
    let orbit_dir = runtime.shared_root();
    let mode = registry
        .checkouts
        .iter()
        .find(|checkout| checkout.orbit_dir == orbit_dir)
        .and_then(|checkout| {
            orbit_remote::workspace_registry::find_workspace(&registry, &checkout.workspace_id)
        })
        .map(orbit_core::resolved_ship_mode)
        .unwrap_or(orbit_core::ShipMode::Local);
    Ok(mode)
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
    mode: orbit_core::ShipMode,
) -> Result<WorkflowRunPlan, OrbitError> {
    validate_task_selection(&args.task_ids)?;
    let workflow_alias = SHIP_WORKFLOW;
    ensure_workflow_exists(workflow_alias)?;
    let base = args.base.as_deref().unwrap_or(config_base_branch);
    Ok(WorkflowRunPlan {
        workflow_alias,
        input: build_ship_input(
            mode,
            base,
            &args.task_ids,
            args.review,
            args.review_crew.as_deref(),
        )?,
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
