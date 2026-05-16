use clap::Args;
use orbit_core::{NotFoundKind, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

use super::job::job_run_to_json_with_state;
use super::steps::{
    filtered_steps, legacy_step_to_json, print_run_header, print_run_header_with_state,
    print_step_record, print_step_summary_table, resolve_run, resolve_run_step,
};

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run\":<job-run>,\"pipeline_state\":<state|null>} or {\"run_id\":...,\"job_id\":...,\"step\":<step>,\"step_output\":<json|null>} with -s.\nExamples:\n  orbit run show\n  orbit run show jrun-20260426-0631\n  orbit run show jrun-20260426-0631 -s implement_one --json"
)]
pub struct RunShowArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Show a single activity step.id from the v2 job YAML; legacy target ID and index still work
    #[arg(short = 's', long = "step")]
    pub step_id: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_show(
            runtime,
            self.run_id.as_deref(),
            self.step_id.as_deref(),
            self.json,
        )
    }
}

pub(crate) fn print_run_show(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let state = runtime.read_run_state(&run.run_id)?;

    if let Some(step_id) = step_id {
        let step = resolve_run_step(runtime, &run, step_id)?;
        let step_output = state
            .as_ref()
            .and_then(|state| state.step_outputs.get(&step.step_index))
            .cloned();
        return print_step_record(&run, &step, step_output, json_output);
    }

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run": job_run_to_json_with_state(&run, state.as_ref()),
            "pipeline_state": state,
        }));
    }

    print_run_header_with_state(&run, state.as_ref());
    if let Some(state) = &state {
        println!(
            "{} iteration={} step_outputs={} updated_at={}",
            crate::output::color::bold("Pipeline:"),
            state.iteration,
            state.step_outputs.len(),
            state.updated_at.to_rfc3339(),
        );
    }
    println!();
    let steps = run.steps.iter().collect::<Vec<_>>();
    print_step_summary_table(&steps)
}

pub(crate) fn print_legacy_logs_summary(
    runtime: &OrbitRuntime,
    run_id: &str,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = runtime
        .show_job_run(run_id)
        .map_err(|_| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
    let steps = filtered_steps(&run, step_id)?;

    if json_output {
        let values = steps
            .iter()
            .map(|step| legacy_step_to_json(step))
            .collect::<Vec<_>>();
        return crate::output::json::print_pretty(&Value::Array(values));
    }

    print_run_header(&run);
    println!();
    print_step_summary_table(&steps)
}
