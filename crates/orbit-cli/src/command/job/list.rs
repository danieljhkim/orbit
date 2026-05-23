use clap::Args;
use orbit_common::types::JobKind;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;

use super::support::{
    format_last_run, job_catalog_filter, job_catalog_target_summary,
    job_catalog_to_json_with_last_run, job_catalog_to_signal_json,
};

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit job list\n  orbit job list --all\n  orbit job list --kind subroutine\n  orbit job list --json"
)]
pub struct JobListArgs {
    /// Include disabled jobs
    #[arg(long)]
    pub all: bool,
    /// Filter to one v2 job kind.
    #[arg(long, value_enum)]
    pub kind: Option<JobKind>,
    /// Output full job objects as JSON
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (job_id, target_id, state only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for JobListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let filter = job_catalog_filter(self.all, self.kind);
        if self.ops {
            let jobs = runtime.list_job_catalog_with_last_run(self.all, filter)?;
            let values = jobs
                .iter()
                .map(|(job, _)| job_catalog_to_signal_json(job))
                .collect::<Vec<_>>();
            return crate::output::json::print_pretty(&Value::Array(values));
        }

        let jobs_with_runs = runtime.list_job_catalog_with_last_run(self.all, filter)?;
        if self.json {
            let values = jobs_with_runs
                .iter()
                .map(|(job, last_run)| job_catalog_to_json_with_last_run(job, last_run.as_ref()))
                .collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&[
                "JOB_ID",
                "KIND",
                "TARGET_TYPE",
                "TARGET_ID",
                "STATE",
                "LAST_RUN",
            ]);
            for (job, last_run) in &jobs_with_runs {
                use comfy_table::Cell;
                let (target_type, target_id) = job_catalog_target_summary(job);
                table.add_row(vec![
                    Cell::new(&job.job_id),
                    Cell::new(job.kind().to_string()),
                    Cell::new(target_type),
                    Cell::new(target_id),
                    crate::output::color::job_state_color_cell(&job.state().to_string()),
                    Cell::new(format_last_run(last_run.as_ref())),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}
