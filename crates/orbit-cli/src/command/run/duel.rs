//! `orbit run duel-plan` CLI entrypoint.

use clap::Args;
use orbit_common::types::AgentFamily;
use orbit_core::{OrbitError, OrbitRuntime, find_workflow};
use serde_json::{Value, json};

use crate::command::Execute;

use super::support::{dispatch_workflow, print_workflow_dispatch_results};

const DUEL_PLAN_WORKFLOW: &str = "duel-plan";

#[derive(Args)]
#[command(
    about = "Run a planning duel for one task",
    override_usage = "orbit run duel-plan <TASK_ID> [OPTIONS]",
    after_help = "Examples:\n  orbit run duel-plan T20260409-0310\n  orbit run duel-plan T20260409-0310 --base main --json\n  orbit run duel-plan T20260409-0310 --wait\n\nBy default this submits the planning-duel pipeline and returns a run ID immediately. Use `orbit run show <RUN_ID>` to inspect it, or pass `--wait` to block until it finishes."
)]
pub struct DuelPlanCommand {
    /// Task ID for the planning duel.
    pub task_id: String,
    /// Base branch for the planning duel pipeline. Defaults to
    /// `[workflow] base_branch` from `config.toml` (or `main` if unset).
    #[arg(short = 'b', long)]
    pub base: Option<String>,
    /// Wait for the planning-duel pipeline to finish before returning.
    #[arg(long)]
    pub wait: bool,
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,
    /// Agent family for planner A role (codex|claude|gemini|grok). Must be supplied together with --planner-b and --arbiter.
    #[arg(long = "planner-a", value_name = "FAMILY")]
    pub planner_a: Option<String>,
    /// Agent family for planner B role (codex|claude|gemini|grok). Must be supplied together with --planner-a and --arbiter.
    #[arg(long = "planner-b", value_name = "FAMILY")]
    pub planner_b: Option<String>,
    /// Agent family for arbiter role (codex|claude|gemini|grok). Must be supplied together with --planner-a and --planner-b.
    #[arg(long, value_name = "FAMILY")]
    pub arbiter: Option<String>,
}

impl Execute for DuelPlanCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_duel_plan_run_plan(
            &self,
            runtime.workflow_base_branch(),
            &runtime.duel_candidate_families(),
        )?;
        let runs = dispatch_workflow(
            runtime,
            plan.workflow_alias,
            &plan.input,
            false,
            plan.wait_for_completion,
            1,
        )?;
        print_workflow_dispatch_results(plan.workflow_alias, &runs, self.json)
    }
}

#[derive(Debug)]
pub(crate) struct DuelPlanRunPlan {
    pub workflow_alias: &'static str,
    pub input: Value,
    pub wait_for_completion: bool,
}

/// Validates explicit --planner-a/--planner-b/--arbiter when any are present.
/// Returns Ok(Some((a,b,arb))) when all three valid and distinct and in candidates;
/// Ok(None) when none present (caller uses random path);
/// Err for partial, bad parse, dup, or not-in-candidates.
fn explicit_duel_role_families(
    args: &DuelPlanCommand,
    candidates: &[String],
) -> Result<Option<(String, String, String)>, OrbitError> {
    let pa = args.planner_a.as_deref();
    let pb = args.planner_b.as_deref();
    let arb = args.arbiter.as_deref();

    let present_count = [pa, pb, arb].iter().filter(|o| o.is_some()).count();
    if present_count == 0 {
        return Ok(None);
    }
    if present_count < 3 {
        let mut missing = Vec::new();
        if pa.is_none() {
            missing.push("--planner-a");
        }
        if pb.is_none() {
            missing.push("--planner-b");
        }
        if arb.is_none() {
            missing.push("--arbiter");
        }
        return Err(OrbitError::InvalidInput(format!(
            "duel-plan explicit roles require all three of --planner-a, --planner-b, --arbiter; missing {}",
            missing.join(", ")
        )));
    }

    let (pa, pb, arb) = match (pa, pb, arb) {
        (Some(pa), Some(pb), Some(arb)) => (pa, pb, arb),
        _ => {
            return Err(OrbitError::InvalidInput(
                "duel-plan explicit roles require all three role flags".to_string(),
            ));
        }
    };

    let fa = AgentFamily::parse(pa)?;
    let fb = AgentFamily::parse(pb)?;
    let fc = AgentFamily::parse(arb)?;

    let sa = fa.as_str();
    let sb = fb.as_str();
    let sc = fc.as_str();

    if sa == sb || sa == sc || sb == sc {
        let dup = if sa == sb || sa == sc { sa } else { sb };
        return Err(OrbitError::InvalidInput(format!(
            "duel-plan explicit roles must use distinct families; '{dup}' appears more than once"
        )));
    }

    for (flag, fam) in [("--planner-a", sa), ("--planner-b", sb), ("--arbiter", sc)] {
        if !candidates.iter().any(|c| c == fam) {
            return Err(OrbitError::InvalidInput(format!(
                "{flag} value '{fam}' is not in [duel] candidates {candidates:?}"
            )));
        }
    }

    Ok(Some((sa.to_string(), sb.to_string(), sc.to_string())))
}

pub(crate) fn build_duel_plan_run_plan(
    args: &DuelPlanCommand,
    config_base_branch: &str,
    duel_candidates: &[String],
) -> Result<DuelPlanRunPlan, OrbitError> {
    find_workflow(DUEL_PLAN_WORKFLOW).ok_or_else(|| {
        OrbitError::InvalidInput(format!("unknown workflow '{DUEL_PLAN_WORKFLOW}'"))
    })?;
    let base = args.base.as_deref().unwrap_or(config_base_branch);

    let input = if let Some((planner_a_family, planner_b_family, arbiter_family)) =
        explicit_duel_role_families(args, duel_candidates)?
    {
        json!({
            "task_id": args.task_id.clone(),
            "task_ids": [args.task_id.clone()],
            "base_branch": base,
            "planner_a_family": planner_a_family,
            "planner_b_family": planner_b_family,
            "arbiter_family": arbiter_family,
        })
    } else {
        json!({
            "task_id": args.task_id.clone(),
            "task_ids": [args.task_id.clone()],
            "base_branch": base,
        })
    };

    Ok(DuelPlanRunPlan {
        workflow_alias: DUEL_PLAN_WORKFLOW,
        input,
        wait_for_completion: args.wait,
    })
}

