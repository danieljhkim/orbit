use clap::{Args, Subcommand};
use orbit_core::{
    OrbitError, OrbitRuntime, WorkflowInput, build_workflow_input_for, find_workflow,
    validate_workflow_flags,
};
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::command::Execute;
use crate::command::job_run_support::{
    RunHistoryFilter, job_run_step_to_json, job_run_to_json_with_workflow, load_filtered_job_runs,
    load_latest_job_run, print_job_run_list_with_workflow, print_job_run_with_workflow,
    print_step_detail, summary_step,
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
    /// Execute the ship pipeline
    Run(ShipRunArgs),
    /// List job runs for ship pipelines
    List(ShipListArgs),
    /// Show a ship pipeline run, or the latest one when no run ID is provided
    Show(ShipShowArgs),
}

impl Execute for ShipSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ShipSubcommand::Run(args) => args.execute(runtime),
            ShipSubcommand::List(args) => args.execute(runtime),
            ShipSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit ship run\n  orbit ship run --tasks T123,T456 --parallelism 2\n  orbit ship run --local --tasks T123 --base main\n  orbit ship run --loop 3"
)]
pub struct ShipRunArgs {
    /// Use the local ship pipeline (`job_local_task_pipeline`) instead of the default PR pipeline.
    #[arg(long)]
    pub local: bool,

    /// Comma-separated task IDs to process (omit to auto-select from backlog)
    #[arg(long)]
    pub tasks: Option<String>,

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

impl Execute for ShipRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let plan = build_ship_run_plan(&self)?;
        let runs = dispatch_workflow(
            runtime,
            plan.workflow_alias,
            &plan.input,
            self.debug,
            plan.loop_count,
        )?;

        if self.json {
            if runs.len() == 1 {
                return crate::output::json::print_pretty(&ship_run_to_json(&runs[0]));
            }
            return crate::output::json::print_pretty(&json!({
                "workflow": plan.workflow_alias,
                "runs": runs.iter().map(ship_run_to_json).collect::<Vec<_>>(),
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

#[derive(Clone)]
struct WorkflowDispatchResult {
    workflow_alias: &'static str,
    job_id: String,
    run_id: String,
    state: String,
    attempt: u32,
    error_code: Option<String>,
    error_message: Option<String>,
}

fn build_ship_run_plan(args: &ShipRunArgs) -> Result<ShipRunPlan, OrbitError> {
    if args.loop_count == 0 {
        return Err(OrbitError::InvalidInput(
            "--loop must be greater than 0".to_string(),
        ));
    }

    validate_explicit_task_selection(args.tasks.as_deref(), args.parallelism)?;

    let workflow_alias = if args.local {
        SHIP_LOCAL_WORKFLOW
    } else {
        SHIP_WORKFLOW
    };
    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;

    let input = WorkflowInput {
        tasks: args.tasks.clone(),
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
    tasks: Option<&str>,
    parallelism: Option<u32>,
) -> Result<(), OrbitError> {
    let Some(tasks) = tasks else {
        return Ok(());
    };

    let task_ids = crate::parse::csv_to_vec(tasks);
    let mut seen = HashSet::new();
    for task_id in &task_ids {
        if !seen.insert(task_id.clone()) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task id '{task_id}' in --tasks"
            )));
        }
    }

    if let Some(parallelism) = parallelism
        && task_ids.len() > parallelism as usize
    {
        return Err(OrbitError::InvalidInput(format!(
            "explicit --tasks batch of {} exceeds --parallelism {}",
            task_ids.len(),
            parallelism
        )));
    }

    Ok(())
}

fn dispatch_workflow(
    runtime: &OrbitRuntime,
    workflow_alias: &'static str,
    input: &Value,
    debug: bool,
    loop_count: u32,
) -> Result<Vec<WorkflowDispatchResult>, OrbitError> {
    let workflow = find_workflow(workflow_alias)
        .ok_or_else(|| OrbitError::InvalidInput(format!("unknown workflow '{workflow_alias}'")))?;

    let mut results = Vec::with_capacity(loop_count as usize);
    for _ in 0..loop_count {
        let run = runtime.run_job_now_with_input_debug(workflow.job_id, input.clone(), debug)?;
        let run_details = runtime
            .job_history(workflow.job_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);
        results.push(WorkflowDispatchResult {
            workflow_alias,
            job_id: run.job_id,
            run_id: run.run_id,
            state: run.state.to_string(),
            attempt: run.attempt,
            error_code: run_details
                .as_ref()
                .and_then(summary_step)
                .and_then(|step| step.error_code.clone()),
            error_message: run_details
                .as_ref()
                .and_then(summary_step)
                .and_then(|step| step.error_message.clone()),
        });
    }

    Ok(results)
}

fn ship_run_to_json(run: &WorkflowDispatchResult) -> Value {
    json!({
        "workflow": run.workflow_alias,
        "job_id": run.job_id,
        "run_id": run.run_id,
        "state": run.state,
        "attempt": run.attempt,
        "error_code": run.error_code,
        "error_message": run.error_message,
    })
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
