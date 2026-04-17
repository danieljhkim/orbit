use clap::{Args, Subcommand};
use orbit_core::{
    OrbitError, OrbitRuntime, WorkflowInput, build_workflow_input_for, find_workflow,
    validate_workflow_flags,
};
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::command::Execute;
use crate::command::job_run_support::{
    RunHistoryFilter, dispatch_workflow, job_run_step_to_json, job_run_to_json_with_workflow,
    load_filtered_job_runs, load_latest_job_run, print_job_run_list_with_workflow,
    print_job_run_with_workflow, print_step_detail, workflow_dispatch_result_to_json,
};

const SHIP_WORKFLOW: &str = "ship";
const SHIP_LOCAL_WORKFLOW: &str = "ship-local";
const SHIP_JOB_ID: &str = "job_parallel_task_pipeline";
const SHIP_LOCAL_JOB_ID: &str = "job_local_task_pipeline";
const SHIP_JOB_IDS: &[&str] = &[SHIP_JOB_ID, SHIP_LOCAL_JOB_ID];

#[derive(Args)]
#[command(
    about = "Ship tasks through the pipeline",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct ShipCommand {
    #[command(subcommand)]
    pub command: ShipSubcommand,
}

impl Execute for ShipCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ShipSubcommand {
    /// Execute the PR-based ship pipeline
    Pr(ShipPrArgs),
    /// Execute the local-only ship pipeline
    Local(ShipLocalArgs),
    /// List job runs for ship pipelines
    List(ShipListArgs),
    /// Show a ship pipeline run, or the latest one when no run ID is provided
    Show(ShipShowArgs),
}

impl Execute for ShipSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ShipSubcommand::Pr(args) => args.execute(runtime),
            ShipSubcommand::Local(args) => args.execute(runtime),
            ShipSubcommand::List(args) => args.execute(runtime),
            ShipSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit ship pr\n  orbit ship pr T123 T456 --parallelism 2\n  orbit ship pr --base main\n  orbit ship pr --loop 3"
)]
pub struct ShipPrArgs {
    #[command(flatten)]
    pub args: ShipWorkflowArgs,
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit ship local\n  orbit ship local T123 --parallelism 1\n  orbit ship local --base main\n  orbit ship local --loop 3"
)]
pub struct ShipLocalArgs {
    #[command(flatten)]
    pub args: ShipWorkflowArgs,
}

#[derive(Args)]
pub struct ShipWorkflowArgs {
    /// Task IDs to process (omit to auto-select from backlog)
    #[arg(value_name = "TASK_IDS", num_args = 1..)]
    pub task_ids: Vec<String>,

    /// Number of parallel workers
    #[arg(long)]
    pub parallelism: Option<u32>,

    /// Base branch for the pipeline
    #[arg(long)]
    pub base: Option<String>,

    /// Repeat the selected ship workflow N times
    #[arg(long = "loop", default_value_t = 1)]
    pub loop_count: u32,

    /// Stream agent stderr to the terminal for debugging
    #[arg(long)]
    pub debug: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ShipPrArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        execute_ship_workflow(runtime, SHIP_WORKFLOW, self.args)
    }
}

impl Execute for ShipLocalArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        execute_ship_workflow(runtime, SHIP_LOCAL_WORKFLOW, self.args)
    }
}

fn execute_ship_workflow(
    runtime: &OrbitRuntime,
    workflow_alias: &'static str,
    args: ShipWorkflowArgs,
) -> Result<(), OrbitError> {
    let plan = build_ship_run_plan(workflow_alias, &args)?;
    let runs = dispatch_workflow(
        runtime,
        plan.workflow_alias,
        &plan.input,
        args.debug,
        plan.loop_count,
    )?;

    if args.json {
        if runs.len() == 1 {
            return crate::output::json::print_pretty(&workflow_dispatch_result_to_json(&runs[0]));
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

#[derive(Args)]
pub struct ShipListArgs {
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

impl Execute for ShipListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = load_filtered_job_runs(
            runtime,
            SHIP_JOB_IDS,
            &RunHistoryFilter {
                status: self.status,
                since: self.since,
                limit: self.limit,
            },
        )?;

        if self.json {
            return crate::output::json::print_pretty(&Value::Array(
                runs.iter()
                    .map(|run| {
                        job_run_to_json_with_workflow(run, ship_workflow_name(run.job_id.as_str()))
                    })
                    .collect::<Vec<_>>(),
            ));
        }

        print_job_run_list_with_workflow(&runs, self.full, ship_workflow_name);
        Ok(())
    }
}

#[derive(Args)]
pub struct ShipShowArgs {
    pub run_id: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub step: Option<usize>,
}

impl Execute for ShipShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = match &self.run_id {
            Some(run_id) => runtime.show_job_run(run_id)?,
            None => load_latest_job_run(runtime, SHIP_JOB_IDS, "ship")?,
        };
        ensure_ship_run(&run)?;

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
                ship_workflow_name(run.job_id.as_str()),
            ));
        }

        print_job_run_with_workflow(&run, ship_workflow_name(run.job_id.as_str()));
        Ok(())
    }
}

struct ShipRunPlan {
    workflow_alias: &'static str,
    input: Value,
    loop_count: u32,
}

fn build_ship_run_plan(
    workflow_alias: &'static str,
    args: &ShipWorkflowArgs,
) -> Result<ShipRunPlan, OrbitError> {
    if args.loop_count == 0 {
        return Err(OrbitError::InvalidInput(
            "--loop must be greater than 0".to_string(),
        ));
    }

    validate_explicit_task_selection(&args.task_ids, args.parallelism)?;

    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;

    let input = WorkflowInput {
        tasks: (!args.task_ids.is_empty()).then(|| args.task_ids.join(",")),
        parallelism: args.parallelism,
        base: args.base.clone(),
        pr_number: None,
    };
    validate_workflow_flags(workflow, &input)?;

    Ok(ShipRunPlan {
        workflow_alias,
        input: build_workflow_input_for(Some(workflow), &input)?,
        loop_count: args.loop_count,
    })
}

fn validate_explicit_task_selection(
    task_ids: &[String],
    parallelism: Option<u32>,
) -> Result<(), OrbitError> {
    if task_ids.is_empty() {
        return Ok(());
    }

    let mut seen = HashSet::new();
    for task_id in task_ids {
        if !seen.insert(task_id.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task id '{task_id}' in explicit task selection"
            )));
        }
    }

    if let Some(parallelism) = parallelism
        && task_ids.len() > parallelism as usize
    {
        return Err(OrbitError::InvalidInput(format!(
            "explicit task batch of {} exceeds --parallelism {}",
            task_ids.len(),
            parallelism
        )));
    }

    Ok(())
}

fn ensure_ship_run(run: &orbit_core::JobRun) -> Result<(), OrbitError> {
    if SHIP_JOB_IDS.contains(&run.job_id.as_str()) {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "run '{}' belongs to job '{}', not a ship pipeline",
        run.run_id, run.job_id
    )))
}

fn ship_workflow_name(job_id: &str) -> Option<&'static str> {
    match job_id {
        SHIP_JOB_ID => Some(SHIP_WORKFLOW),
        SHIP_LOCAL_JOB_ID => Some(SHIP_LOCAL_WORKFLOW),
        _ => None,
    }
}
