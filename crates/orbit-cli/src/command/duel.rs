//! `orbit duel` CLI subcommand tree.
//!
//! Thin presentation layer over `orbit_core::duel_scoreboard::aggregate`.
//! All math lives in the store crate (re-exported via orbit-core) so tests
//! and future programmatic callers can reach the same numbers without
//! reimplementing anything.

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::duel_scoreboard::{
    AggregateFilter, AggregateRow, Aggregates, RoleAxis, SegmentBy, aggregate,
};
use orbit_core::{
    OrbitError, OrbitRuntime, WorkflowInput, build_workflow_input_for, find_workflow,
    validate_workflow_flags,
};
use orbit_types::DuelRun;
use serde_json::{Value, json};

use crate::command::Execute;
use crate::command::job_run_support::{
    RunHistoryFilter, dispatch_workflow, job_run_step_to_json, job_run_to_json_with_workflow,
    load_filtered_job_runs, load_latest_job_run, print_job_run_list_with_workflow,
    print_job_run_with_workflow, print_step_detail, summary_step, workflow_dispatch_result_to_json,
};

const DUEL_PR_WORKFLOW: &str = "duel";
const DUEL_PLAN_WORKFLOW: &str = "duel-plan";
const DUEL_JOB_IDS: &[&str] = &["job_duel_pipeline", "job_duel_plan_pipeline"];

#[derive(Args)]
#[command(
    about = "Cross-agent scoring and planning",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct DuelCommand {
    #[command(subcommand)]
    pub command: DuelSubcommand,
}

impl Execute for DuelCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum DuelSubcommand {
    /// Run a single-task PR duel
    Pr(DuelPrArgs),
    /// Run a single-task planning duel
    Plan(DuelPlanArgs),
    /// Show scoreboard aggregates computed from `.orbit/state/scoreboard/duel.json`.
    #[command(alias = "scoreboard")]
    Score(DuelScoreboardArgs),
    /// List duel job runs
    List(DuelListArgs),
    /// Show a duel run, or the latest one when no run ID is provided
    Show(DuelShowArgs),
}

impl Execute for DuelSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            DuelSubcommand::Pr(args) => args.execute(runtime),
            DuelSubcommand::Plan(args) => args.execute(runtime),
            DuelSubcommand::Score(args) => args.execute(runtime),
            DuelSubcommand::List(args) => args.execute(runtime),
            DuelSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit duel pr\n  orbit duel pr --loop 3\n  orbit duel pr T20260409-0310\n  orbit duel pr T20260409-0310 --base main --json"
)]
pub struct DuelPrArgs {
    /// Optional task ID. Omit to auto-select the first available duel-eligible task.
    #[arg(value_name = "TASK_ID", num_args = 0..=1)]
    pub task_id: Option<String>,
    /// Base branch for the duel pipeline
    #[arg(long)]
    pub base: Option<String>,
    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Repeat the selected duel workflow N times
    #[arg(long = "loop")]
    pub loop_count: Option<u32>,
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit duel plan T20260409-0310\n  orbit duel plan T20260409-0310 --base main --json"
)]
pub struct DuelPlanArgs {
    /// Task ID for the planning duel.
    pub task_id: String,
    /// Base branch for the planning duel pipeline
    #[arg(long)]
    pub base: Option<String>,
    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelPrArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_duel_pr_run_plan(&self)?;
        let runs = dispatch_workflow(
            runtime,
            plan.workflow_alias,
            &plan.input,
            self.debug,
            plan.loop_count,
        )?;

        if self.json {
            if runs.len() == 1 {
                return crate::output::json::print_pretty(&workflow_dispatch_result_to_json(
                    &runs[0],
                ));
            }
            return crate::output::json::print_pretty(&json!({
                "workflow": plan.workflow_alias,
                "runs": runs
                    .iter()
                    .map(workflow_dispatch_result_to_json)
                    .collect::<Vec<_>>(),
            }));
        }

        for run in &runs {
            let error_code = run.error_code.clone().unwrap_or_else(|| "-".to_string());
            let error_message = run
                .error_message
                .clone()
                .unwrap_or_else(|| "-".to_string())
                .replace('\n', " ");
            println!(
                "workflow={};job_id={};run_id={};state={};attempt={};error_code={};error_message={}",
                run.workflow_alias,
                run.job_id,
                run.run_id,
                run.state,
                run.attempt,
                error_code,
                error_message
            );
        }
        Ok(())
    }
}

impl Execute for DuelPlanArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        execute_duel_workflow(
            runtime,
            DUEL_PLAN_WORKFLOW,
            Some(self.task_id),
            self.base,
            self.debug,
            self.json,
        )
    }
}

fn execute_duel_workflow(
    runtime: &OrbitRuntime,
    workflow_alias: &str,
    task_id: Option<String>,
    base: Option<String>,
    debug: bool,
    json: bool,
) -> Result<(), OrbitError> {
    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;
    let input = WorkflowInput {
        tasks: task_id,
        parallelism: None,
        base,
        pr_number: None,
    };
    validate_workflow_flags(workflow, &input)?;
    let built_input = build_workflow_input_for(Some(workflow), &input)?;
    let run = runtime.run_job_now_with_input_debug(workflow.job_id, built_input, debug)?;
    let run_details = runtime
        .job_history(workflow.job_id)?
        .into_iter()
        .find(|entry| entry.run_id == run.run_id);

    if json {
        return crate::output::json::print_pretty(&json!({
            "workflow": workflow.alias,
            "job_id": run.job_id,
            "run_id": run.run_id,
            "state": run.state.to_string(),
            "attempt": run.attempt,
            "error_code": run_details.as_ref().and_then(summary_step).and_then(|step| step.error_code.clone()),
            "error_message": run_details.as_ref().and_then(summary_step).and_then(|step| step.error_message.clone()),
        }));
    }

    let error_code = run_details
        .as_ref()
        .and_then(summary_step)
        .and_then(|step| step.error_code.clone())
        .unwrap_or_else(|| "-".to_string());
    let error_message = run_details
        .as_ref()
        .and_then(summary_step)
        .and_then(|step| step.error_message.clone())
        .unwrap_or_else(|| "-".to_string())
        .replace('\n', " ");
    println!(
        "workflow={};job_id={};run_id={};state={};attempt={};error_code={};error_message={}",
        workflow.alias, run.job_id, run.run_id, run.state, run.attempt, error_code, error_message
    );
    Ok(())
}

struct DuelRunPlan {
    workflow_alias: &'static str,
    input: Value,
    loop_count: u32,
}

fn build_duel_pr_run_plan(args: &DuelPrArgs) -> Result<DuelRunPlan, OrbitError> {
    let loop_count = args.loop_count.unwrap_or(1);
    if loop_count == 0 {
        return Err(OrbitError::InvalidInput(
            "--loop must be greater than 0".to_string(),
        ));
    }
    if args.loop_count.is_some() && args.task_id.is_some() {
        return Err(OrbitError::InvalidInput(
            "--loop cannot be combined with an explicit task id; omit [TASK_ID] to auto-select each iteration".to_string(),
        ));
    }

    let workflow = find_workflow(DUEL_PR_WORKFLOW).ok_or_else(|| {
        OrbitError::InvalidInput(format!("unknown workflow '{DUEL_PR_WORKFLOW}'"))
    })?;
    let input = WorkflowInput {
        tasks: args.task_id.clone(),
        parallelism: None,
        base: args.base.clone(),
        pr_number: None,
    };
    validate_workflow_flags(workflow, &input)?;

    Ok(DuelRunPlan {
        workflow_alias: DUEL_PR_WORKFLOW,
        input: build_workflow_input_for(Some(workflow), &input)?,
        loop_count,
    })
}

/// How the flat table should be sliced before display.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SegmentByArg {
    /// No segmentation — one row per (role, agent, model).
    None,
    /// Segment by `task_class.scope` (single_file / multi_file / cross_crate / other).
    Scope,
    /// Segment by `task_class.ambiguity` (well_specified / needs_judgment / exploratory / unknown).
    Ambiguity,
}

impl From<SegmentByArg> for SegmentBy {
    fn from(value: SegmentByArg) -> Self {
        match value {
            SegmentByArg::None => SegmentBy::None,
            SegmentByArg::Scope => SegmentBy::Scope,
            SegmentByArg::Ambiguity => SegmentBy::Ambiguity,
        }
    }
}

/// Role filter — mirrors [`RoleAxis`] but adds `All` as the default.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum RoleFilterArg {
    /// All three roles (default).
    All,
    Implementer,
    Reviewer,
    Arbiter,
}

impl RoleFilterArg {
    fn into_filter(self) -> Option<RoleAxis> {
        match self {
            RoleFilterArg::All => None,
            RoleFilterArg::Implementer => Some(RoleAxis::Implementer),
            RoleFilterArg::Reviewer => Some(RoleAxis::Reviewer),
            RoleFilterArg::Arbiter => Some(RoleAxis::Arbiter),
        }
    }
}

#[derive(Args)]
pub struct DuelScoreboardArgs {
    /// Segment the table by a `task_class` dimension.
    #[arg(long, value_enum, default_value_t = SegmentByArg::None)]
    pub by: SegmentByArg,
    /// Filter to a single role. Defaults to showing all three roles.
    #[arg(long, value_enum, default_value_t = RoleFilterArg::All)]
    pub role: RoleFilterArg,
    /// Emit raw aggregates as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelScoreboardArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = runtime.load_duel_runs()?;
        let filter = AggregateFilter {
            segment_by: self.by.into(),
            role: self.role.into_filter(),
        };
        let aggs = aggregate(&runs, filter);

        if self.json {
            return emit_json(&runs, &aggs);
        }
        render_table(&runs, &aggs);
        Ok(())
    }
}

#[derive(Args)]
pub struct DuelListArgs {
    #[arg(long, value_enum)]
    pub status: Option<orbit_core::JobRunState>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub full: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for DuelListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = load_filtered_job_runs(
            runtime,
            DUEL_JOB_IDS,
            &RunHistoryFilter {
                status: self.status,
                since: self.since,
                limit: self.limit,
            },
        )?;

        if self.json {
            return crate::output::json::print_pretty(&serde_json::Value::Array(
                runs.iter()
                    .map(|run| {
                        job_run_to_json_with_workflow(run, duel_workflow_name(run.job_id.as_str()))
                    })
                    .collect::<Vec<_>>(),
            ));
        }

        print_job_run_list_with_workflow(&runs, self.full, duel_workflow_name);
        Ok(())
    }
}

#[derive(Args)]
pub struct DuelShowArgs {
    pub run_id: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub step: Option<usize>,
}

impl Execute for DuelShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = match &self.run_id {
            Some(run_id) => runtime.show_job_run(run_id)?,
            None => load_latest_job_run(runtime, DUEL_JOB_IDS, "duel")?,
        };
        ensure_duel_run(&run)?;

        if let Some(step_index) = self.step {
            let step = run
                .steps
                .iter()
                .find(|step| step.step_index as usize == step_index)
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "step {step_index} not found in run '{}' (run has {} step(s))",
                        run.run_id,
                        run.steps.len()
                    ))
                })?;
            if self.json {
                return crate::output::json::print_pretty(&job_run_step_to_json(step));
            }
            print_step_detail(step);
            return Ok(());
        }

        if self.json {
            return crate::output::json::print_pretty(&job_run_to_json_with_workflow(
                &run,
                duel_workflow_name(run.job_id.as_str()),
            ));
        }

        print_job_run_with_workflow(&run, duel_workflow_name(run.job_id.as_str()));
        Ok(())
    }
}

fn emit_json(runs: &[DuelRun], aggs: &Aggregates) -> Result<(), OrbitError> {
    let payload = serde_json::json!({
        "runs": runs.len(),
        "rows": aggs.rows.iter().map(row_to_json).collect::<Vec<_>>(),
    });
    crate::output::json::print_pretty(&payload)
}

fn row_to_json(row: &AggregateRow) -> serde_json::Value {
    serde_json::json!({
        "segment": row.segment,
        "role": row.role,
        "agent": row.agent,
        "model": row.model,
        "runs": row.runs,
        "avg_score": row.avg_score,
        "merge_rate": row.merge_rate,
        "avg_fix_iterations": row.avg_fix_iterations,
        "avg_wall_seconds": row.avg_wall_seconds,
    })
}

fn render_table(runs: &[DuelRun], aggs: &Aggregates) {
    if runs.is_empty() {
        println!("No duel runs recorded yet.");
        return;
    }
    if aggs.rows.is_empty() {
        println!("No rows match the selected filters (runs={}).", runs.len());
        return;
    }

    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&[
        "SEGMENT",
        "ROLE",
        "AGENT/MODEL",
        "RUNS",
        "AVG SCORE",
        "MERGE RATE",
        "AVG FIX ITERS",
        "AVG WALL SECS",
    ]);
    for row in &aggs.rows {
        table.add_row(vec![
            Cell::new(&row.segment),
            Cell::new(row.role),
            Cell::new(format!("{} / {}", row.agent, row.model)),
            Cell::new(row.runs),
            Cell::new(format!("{:.2}", row.avg_score)),
            Cell::new(format!("{:.2}", row.merge_rate)),
            Cell::new(format!("{:.2}", row.avg_fix_iterations)),
            Cell::new(format!("{:.0}", row.avg_wall_seconds)),
        ]);
    }
    println!("{table}");
}

fn ensure_duel_run(run: &orbit_core::JobRun) -> Result<(), OrbitError> {
    if DUEL_JOB_IDS.contains(&run.job_id.as_str()) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "run '{}' belongs to job '{}', not a duel pipeline",
        run.run_id, run.job_id
    )))
}

fn duel_workflow_name(job_id: &str) -> Option<&'static str> {
    match job_id {
        "job_duel_pipeline" => Some(DUEL_PR_WORKFLOW),
        "job_duel_plan_pipeline" => Some(DUEL_PLAN_WORKFLOW),
        _ => None,
    }
}
