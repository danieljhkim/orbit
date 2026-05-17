//! `orbit run ship` CLI entrypoint.

use std::collections::HashSet;

use clap::{Args, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime, find_workflow};
use serde_json::Value;

use crate::command::Execute;

use super::support::{dispatch_workflow, print_workflow_dispatch_results};

const SHIP_WORKFLOW: &str = "ship";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ShipMode {
    Pr,
    Local,
}

impl ShipMode {
    fn as_input_value(self) -> &'static str {
        match self {
            ShipMode::Pr => "pr",
            ShipMode::Local => "local",
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
    about = "Deprecated alias for `orbit run ship`",
    override_usage = "orbit run ship-auto [OPTIONS]",
    after_help = "`orbit run ship-auto` was replaced by `orbit run ship`. Omit task ids for auto mode."
)]
pub struct LegacyShipAutoCommand {
    /// Deprecated. Use `orbit run ship --mode <MODE>`.
    #[arg(short = 'm', long, value_enum, default_value = "pr")]
    pub mode: ShipMode,
    /// Deprecated. Use `orbit run ship --base <BRANCH>`.
    #[arg(short = 'b', long)]
    pub base: Option<String>,
    /// Deprecated.
    #[arg(long)]
    pub json: bool,
}

impl Execute for LegacyShipAutoCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let _ = self;
        Err(OrbitError::InvalidInput(
            "`orbit run ship-auto` was replaced by `orbit run ship` (auto mode runs when no task ids are supplied)".to_string(),
        ))
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
        input: ship_input(args.mode, base, &args.task_ids),
    })
}

fn ship_input(mode: ShipMode, base: &str, task_ids: &[String]) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "mode".to_string(),
        Value::String(mode.as_input_value().to_string()),
    );
    map.insert("base_branch".to_string(), Value::String(base.to_string()));
    if !task_ids.is_empty() {
        map.insert(
            "task_ids".to_string(),
            Value::Array(task_ids.iter().cloned().map(Value::String).collect()),
        );
    }
    Value::Object(map)
}

fn validate_task_selection(task_ids: &[String]) -> Result<(), OrbitError> {
    if let Some(legacy) = task_ids.first().and_then(|value| legacy_ship_form(value)) {
        return Err(OrbitError::InvalidInput(legacy.to_string()));
    }

    let mut seen = HashSet::new();
    for task_id in task_ids {
        if !seen.insert(task_id.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task id '{task_id}' in explicit task selection"
            )));
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ship_args(task_ids: &[&str], mode: ShipMode, base: Option<&str>) -> ShipCommand {
        ShipCommand {
            task_ids: task_ids.iter().map(|value| value.to_string()).collect(),
            mode,
            base: base.map(str::to_string),
            json: false,
        }
    }

    #[test]
    fn ship_auto_mode_omits_task_ids_and_uses_pr_mode_by_default() {
        let plan = build_ship_run_plan(&ship_args(&[], ShipMode::Pr, None), "agent-main")
            .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "mode": "pr",
                "base_branch": "agent-main",
            })
        );
    }

    #[test]
    fn ship_auto_mode_preserves_local_mode_and_base_override() {
        let plan =
            build_ship_run_plan(&ship_args(&[], ShipMode::Local, Some("main")), "agent-main")
                .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "mode": "local",
                "base_branch": "main",
            })
        );
    }

    #[test]
    fn explicit_ship_uses_unified_gated_workflow_with_pr_mode() {
        let plan = build_ship_run_plan(
            &ship_args(&["T20260425-2010", "T20260425-2011"], ShipMode::Pr, None),
            "agent-main",
        )
        .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "mode": "pr",
                "base_branch": "agent-main",
                "task_ids": ["T20260425-2010", "T20260425-2011"],
            })
        );
    }

    #[test]
    fn explicit_ship_preserves_local_mode_and_base_override() {
        let plan = build_ship_run_plan(
            &ship_args(&["T20260425-2010"], ShipMode::Local, Some("main")),
            "agent-main",
        )
        .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "mode": "local",
                "base_branch": "main",
                "task_ids": ["T20260425-2010"],
            })
        );
    }

    #[test]
    fn ship_auto_deprecation_returns_legacy_error() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = LegacyShipAutoCommand {
            mode: ShipMode::Pr,
            base: None,
            json: false,
        }
        .execute(&runtime)
        .expect_err("deprecated command should fail");
        assert!(
            err.to_string().contains("orbit run ship"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_local_deprecation_returns_legacy_error() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = LegacyShipLocalCommand {
            task_ids: vec!["T20260425-2010".to_string()],
            base: None,
            json: false,
        }
        .execute(&runtime)
        .expect_err("deprecated command should fail");
        assert!(
            err.to_string().contains("orbit run ship --mode local"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_removed_history_forms() {
        let err = build_ship_run_plan(&ship_args(&["list"], ShipMode::Pr, None), "agent-main")
            .expect_err("legacy history form should fail");
        assert!(
            err.to_string().contains("orbit run history"),
            "unexpected error: {err}"
        );

        let err = build_ship_run_plan(&ship_args(&["show"], ShipMode::Pr, None), "agent-main")
            .expect_err("legacy history form should fail");
        assert!(
            err.to_string().contains("orbit run history"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_removed_local_subcommand_form() {
        let err = build_ship_run_plan(&ship_args(&["local"], ShipMode::Pr, None), "agent-main")
            .expect_err("legacy local form should fail");
        assert!(
            err.to_string().contains("--mode local"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_removed_auto_positional_form() {
        let err = build_ship_run_plan(&ship_args(&["auto"], ShipMode::Pr, None), "agent-main")
            .expect_err("legacy auto form should fail");
        assert!(
            err.to_string().contains("orbit run ship"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_duplicate_task_ids() {
        let err = build_ship_run_plan(
            &ship_args(&["T20260425-2010", "T20260425-2010"], ShipMode::Pr, None),
            "agent-main",
        )
        .expect_err("duplicate task IDs should fail");
        assert!(
            err.to_string().contains("duplicate task id"),
            "unexpected error: {err}"
        );
    }
}
