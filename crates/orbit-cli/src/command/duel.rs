//! `orbit run duel-plan` CLI entrypoint.

use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, find_workflow};
use serde_json::{Value, json};

use crate::command::Execute;
use crate::command::job_run_support::{dispatch_workflow, print_workflow_dispatch_results};

const DUEL_PLAN_WORKFLOW: &str = "duel-plan";

#[derive(Args)]
#[command(
    about = "Run a planning duel for one task",
    override_usage = "orbit run duel-plan <TASK_ID> [OPTIONS]",
    after_help = "Examples:\n  orbit run duel-plan T20260409-0310\n  orbit run duel-plan T20260409-0310 --base main --json"
)]
pub struct DuelPlanCommand {
    /// Task ID for the planning duel.
    pub task_id: String,
    /// Base branch for the planning duel pipeline.
    #[arg(short = 'b', long, default_value = "agent-main")]
    pub base: String,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelPlanCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_duel_plan_run_plan(&self)?;
        let runs = dispatch_workflow(runtime, plan.workflow_alias, &plan.input, false, 1)?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

#[derive(Debug)]
pub(crate) struct DuelPlanRunPlan {
    pub workflow_alias: &'static str,
    pub input: Value,
}

pub(crate) fn build_duel_plan_run_plan(
    args: &DuelPlanCommand,
) -> Result<DuelPlanRunPlan, OrbitError> {
    find_workflow(DUEL_PLAN_WORKFLOW).ok_or_else(|| {
        OrbitError::InvalidInput(format!("unknown workflow '{DUEL_PLAN_WORKFLOW}'"))
    })?;
    Ok(DuelPlanRunPlan {
        workflow_alias: DUEL_PLAN_WORKFLOW,
        input: json!({
            "task_id": args.task_id.clone(),
            "task_ids": [args.task_id.clone()],
            "base_branch": args.base.clone(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn duel_plan_builds_persisted_workflow_input() {
        let plan = build_duel_plan_run_plan(&DuelPlanCommand {
            task_id: "T20260425-2010".to_string(),
            base: "main".to_string(),
            json: false,
        })
        .expect("build duel-plan run plan");

        assert_eq!(plan.workflow_alias, DUEL_PLAN_WORKFLOW);
        assert_eq!(
            plan.input,
            json!({
                "task_id": "T20260425-2010",
                "task_ids": ["T20260425-2010"],
                "base_branch": "main",
            })
        );
    }
}
