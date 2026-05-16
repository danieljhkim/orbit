use clap::Args;
use orbit_core::command::job::JobRunListParams;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

use super::format::{format_timestamp, format_waiting_line, summarize_error_message};
use super::job::job_run_to_json_with_state;

pub(crate) const DEFAULT_HISTORY_LIMIT: usize = 50;

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"runs\":[<job-run>]}\nExamples:\n  orbit run history\n  orbit run history -j task_local_pipeline --limit 20\n  orbit run history --json"
)]
pub struct RunHistoryArgs {
    /// Filter to one job ID
    #[arg(short = 'j', long = "job")]
    pub job_id: Option<String>,

    /// Maximum number of runs to show
    #[arg(long, default_value_t = DEFAULT_HISTORY_LIMIT)]
    pub limit: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_history(runtime, self.job_id.as_deref(), Some(self.limit), self.json)
    }
}

pub(crate) fn print_run_history(
    runtime: &OrbitRuntime,
    job_id: Option<&str>,
    limit: Option<usize>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let runs = match job_id {
        Some(job_id) => runtime.list_job_runs(JobRunListParams {
            job_id: Some(job_id.to_string()),
            limit,
            ..Default::default()
        })?,
        None => runtime.list_job_runs(JobRunListParams {
            limit,
            ..Default::default()
        })?,
    };

    let states = runs
        .iter()
        .map(|run| runtime.read_run_state(&run.run_id))
        .collect::<Result<Vec<_>, _>>()?;

    if json_output {
        let values = runs
            .iter()
            .zip(states.iter())
            .map(|(run, state)| job_run_to_json_with_state(run, state.as_ref()))
            .collect::<Vec<_>>();
        return crate::output::json::print_pretty(&json!({ "runs": values }));
    }

    let include_job_id = job_id.is_none();
    let headers = if include_job_id {
        vec![
            "RUN_ID",
            "JOB_ID",
            "ATTEMPT",
            "STATE",
            "STARTED_AT",
            "FINISHED_AT",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]
    } else {
        vec![
            "RUN_ID",
            "ATTEMPT",
            "STATE",
            "STARTED_AT",
            "FINISHED_AT",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]
    };
    let mut table = crate::output::table::build_table(&headers);
    for run in &runs {
        use comfy_table::Cell;
        let last = run.steps.last();
        let mut row = vec![Cell::new(&run.run_id)];
        if include_job_id {
            row.push(Cell::new(&run.job_id));
        }
        row.extend([
            Cell::new(run.attempt.to_string()),
            crate::output::color::job_state_color_cell(&run.state.to_string()),
            Cell::new(format_timestamp(run.started_at)),
            Cell::new(format_timestamp(run.finished_at)),
            Cell::new(last.and_then(|s| s.error_code.as_deref()).unwrap_or("-")),
            Cell::new(summarize_error_message(
                last.and_then(|s| s.error_message.as_deref()),
            )),
        ]);
        table.add_row(row);
    }
    println!("{table}");
    for (run, state) in runs.iter().zip(states.iter()) {
        if let Some(line) = format_waiting_line(run.state, state.as_ref()) {
            println!("{line}");
        }
    }
    Ok(())
}
