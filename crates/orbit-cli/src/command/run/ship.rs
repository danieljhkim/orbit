//! `orbit run ship` and `orbit run ship-auto` CLI entrypoints.

use std::collections::HashSet;

use clap::{Args, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime, find_workflow};
use serde_json::Value;

use crate::command::Execute;

use super::support::{dispatch_workflow, print_workflow_dispatch_results};

const SHIP_PR_WORKFLOW: &str = "ship";
const SHIP_LOCAL_WORKFLOW: &str = "ship-local";
const SHIP_AUTO_WORKFLOW: &str = "ship-auto";

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

    fn explicit_workflow_alias(self) -> &'static str {
        match self {
            ShipMode::Pr => SHIP_PR_WORKFLOW,
            ShipMode::Local => SHIP_LOCAL_WORKFLOW,
        }
    }
}

#[derive(Args)]
#[command(
    about = "Ship explicitly selected tasks through the task pipeline",
    override_usage = "orbit run ship <TASK_ID> [<TASK_ID>...] [OPTIONS]",
    after_help = "Examples:\n  orbit run ship T123\n  orbit run ship T123 T456 --mode local\n  orbit run ship T123 --base main\n\nRun history moved to `orbit run history -j <JOB_ID>`."
)]
pub struct ShipCommand {
    /// Task IDs to process as one explicit task bundle.
    #[arg(value_name = "TASK_ID", num_args = 0..)]
    pub task_ids: Vec<String>,
    /// Pipeline mode for the selected task bundle.
    #[arg(short = 'm', long, value_enum, default_value = "pr")]
    pub mode: ShipMode,
    /// Base branch for the selected pipeline.
    #[arg(short = 'b', long, default_value = "agent-main")]
    pub base: String,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for ShipCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_ship_run_plan(&self)?;
        let runs = dispatch_workflow(runtime, plan.workflow_alias, &plan.input, false, 1)?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

#[derive(Args)]
#[command(
    about = "Auto-select backlog tasks and ship them through the task pipeline",
    override_usage = "orbit run ship-auto [OPTIONS]",
    after_help = "Examples:\n  orbit run ship-auto\n  orbit run ship-auto --mode local\n  orbit run ship-auto --base main\n\nOutput status labels: empty_backlog, gated_noop, gate_waiting, gate_failed, completed. Gated/no-op statuses keep exit code 0 and are reported explicitly in text and JSON output.\n\nRun history moved to `orbit run history -j task_auto_pipeline`."
)]
pub struct ShipAutoCommand {
    /// Pipeline mode for auto-selected task bundles.
    #[arg(short = 'm', long, value_enum, default_value = "pr")]
    pub mode: ShipMode,
    /// Base branch for auto-selected task bundles.
    #[arg(short = 'b', long, default_value = "agent-main")]
    pub base: String,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for ShipAutoCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_ship_auto_run_plan(&self)?;
        let runs = dispatch_workflow(runtime, plan.workflow_alias, &plan.input, false, 1)?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

#[derive(Debug)]
pub(crate) struct WorkflowRunPlan {
    pub workflow_alias: &'static str,
    pub input: Value,
}

pub(crate) fn build_ship_run_plan(args: &ShipCommand) -> Result<WorkflowRunPlan, OrbitError> {
    validate_explicit_task_selection(&args.task_ids)?;
    let workflow_alias = args.mode.explicit_workflow_alias();
    ensure_workflow_exists(workflow_alias)?;
    Ok(WorkflowRunPlan {
        workflow_alias,
        input: ship_input(args.mode, &args.base, Some(&args.task_ids)),
    })
}

pub(crate) fn build_ship_auto_run_plan(
    args: &ShipAutoCommand,
) -> Result<WorkflowRunPlan, OrbitError> {
    ensure_workflow_exists(SHIP_AUTO_WORKFLOW)?;
    Ok(WorkflowRunPlan {
        workflow_alias: SHIP_AUTO_WORKFLOW,
        input: ship_input(args.mode, &args.base, None),
    })
}

fn ship_input(mode: ShipMode, base: &str, task_ids: Option<&[String]>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "mode".to_string(),
        Value::String(mode.as_input_value().to_string()),
    );
    map.insert("base_branch".to_string(), Value::String(base.to_string()));
    if let Some(task_ids) = task_ids {
        map.insert(
            "task_ids".to_string(),
            Value::Array(task_ids.iter().cloned().map(Value::String).collect()),
        );
    }
    Value::Object(map)
}

fn validate_explicit_task_selection(task_ids: &[String]) -> Result<(), OrbitError> {
    if task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`orbit run ship` requires at least one task ID; use `orbit run ship-auto` to auto-select backlog tasks".to_string(),
        ));
    }

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

    fn explicit_args(task_ids: &[&str], mode: ShipMode, base: &str) -> ShipCommand {
        ShipCommand {
            task_ids: task_ids.iter().map(|value| value.to_string()).collect(),
            mode,
            base: base.to_string(),
            json: false,
        }
    }

    fn auto_args(mode: ShipMode, base: &str) -> ShipAutoCommand {
        ShipAutoCommand {
            mode,
            base: base.to_string(),
            json: false,
        }
    }

    #[test]
    fn explicit_ship_defaults_to_pr_job_input_shape() {
        let plan = build_ship_run_plan(&explicit_args(
            &["T20260425-2010", "T20260425-2011"],
            ShipMode::Pr,
            "agent-main",
        ))
        .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_PR_WORKFLOW);
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
    fn explicit_ship_local_mode_uses_local_job_input_shape() {
        let plan =
            build_ship_run_plan(&explicit_args(&["T20260425-2010"], ShipMode::Local, "main"))
                .expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_LOCAL_WORKFLOW);
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
    fn ship_auto_uses_auto_job_without_explicit_task_ids() {
        let plan =
            build_ship_auto_run_plan(&auto_args(ShipMode::Pr, "agent-main")).expect("build plan");

        assert_eq!(plan.workflow_alias, SHIP_AUTO_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "mode": "pr",
                "base_branch": "agent-main",
            })
        );
    }

    #[test]
    fn ship_rejects_removed_history_forms() {
        let err = build_ship_run_plan(&explicit_args(&["list"], ShipMode::Pr, "agent-main"))
            .expect_err("legacy history form should fail");
        assert!(
            err.to_string().contains("orbit run history"),
            "unexpected error: {err}"
        );

        let err = build_ship_run_plan(&explicit_args(&["show"], ShipMode::Pr, "agent-main"))
            .expect_err("legacy history form should fail");
        assert!(
            err.to_string().contains("orbit run history"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_removed_local_subcommand_form() {
        let err = build_ship_run_plan(&explicit_args(&["local"], ShipMode::Pr, "agent-main"))
            .expect_err("legacy local form should fail");
        assert!(
            err.to_string().contains("--mode local"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_requires_explicit_task_ids() {
        let err = build_ship_run_plan(&explicit_args(&[], ShipMode::Pr, "agent-main"))
            .expect_err("missing explicit task IDs should fail");
        assert!(
            err.to_string().contains("ship-auto"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ship_rejects_duplicate_task_ids() {
        let err = build_ship_run_plan(&explicit_args(
            &["T20260425-2010", "T20260425-2010"],
            ShipMode::Pr,
            "agent-main",
        ))
        .expect_err("duplicate task IDs should fail");
        assert!(
            err.to_string().contains("duplicate task id"),
            "unexpected error: {err}"
        );
    }
}
