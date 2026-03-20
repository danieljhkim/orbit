use chrono::{Duration, Utc};
use clap::{Args, Subcommand};
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::{JobRun, JobRunState, OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;
use crate::command::job::{job_run_to_json, summarize_error_message};

#[derive(Args)]
pub struct JobRunCommand {
    #[command(subcommand)]
    pub command: JobRunSubcommand,
}

impl Execute for JobRunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum JobRunSubcommand {
    List(JobRunListArgs),
    Show(JobRunShowArgs),
    Archive(JobRunArchiveArgs),
    Delete(JobRunDeleteArgs),
}

impl Execute for JobRunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobRunSubcommand::List(args) => args.execute(runtime),
            JobRunSubcommand::Show(args) => args.execute(runtime),
            JobRunSubcommand::Archive(args) => args.execute(runtime),
            JobRunSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct JobRunListArgs {
    #[arg(long)]
    pub job: Option<String>,
    #[arg(long, value_enum)]
    pub status: Option<JobRunState>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobRunListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let since = self
            .since
            .as_deref()
            .map(crate::parse::parse_duration_seconds)
            .transpose()?
            .map(|seconds| Utc::now() - Duration::seconds(seconds as i64));

        let runs = runtime.list_job_runs(JobRunListParams {
            job_id: self.job,
            state: self.status,
            since,
            limit: self.limit,
        })?;

        if self.json {
            let values = runs.iter().map(job_run_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&[
                "RUN_ID",
                "JOB_ID",
                "ATTEMPT",
                "STATE",
                "STARTED_AT",
                "FINISHED_AT",
                "ERROR_CODE",
                "ERROR_MESSAGE",
            ]);
            for run in &runs {
                use comfy_table::Cell;
                table.add_row(vec![
                    Cell::new(&run.run_id),
                    Cell::new(&run.job_id),
                    Cell::new(&run.attempt.to_string()),
                    crate::output::color::job_state_color_cell(&run.state.to_string()),
                    Cell::new(
                        &run.started_at
                            .map(|value| value.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(
                        &run.finished_at
                            .map(|value| value.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(
                        &run.steps
                            .last()
                            .and_then(|s| s.error_code.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(&summarize_error_message(
                        run.steps.last().and_then(|s| s.error_message.as_deref()),
                    )),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunShowArgs {
    pub run_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobRunShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime.show_job_run(&self.run_id)?;
        if self.json {
            crate::output::json::print_pretty(&job_run_to_json(&run))
        } else {
            print_job_run(&run);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunArchiveArgs {
    pub run_id: String,
}

impl Execute for JobRunArchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.archive_job_run(&self.run_id)?;
        println!("Archived job run '{}'", self.run_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct JobRunDeleteArgs {
    pub run_id: String,
}

impl Execute for JobRunDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_job_run(&self.run_id)?;
        println!("Deleted job run '{}'", self.run_id);
        Ok(())
    }
}

fn print_job_run(run: &JobRun) {
    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Run ID:"), run.run_id);
    println!("{} {}", bold("Job ID:"), run.job_id);
    println!("{} {}", bold("Attempt:"), run.attempt);
    println!(
        "{} {}",
        bold("State:"),
        job_state_color(&run.state.to_string())
    );
    println!(
        "{} {}",
        bold("Scheduled:"),
        dimmed(&run.scheduled_at.to_rfc3339())
    );
    println!(
        "{} {}",
        bold("Started:"),
        run.started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Finished:"),
        run.finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Duration (ms):"),
        run.duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    let last_step = run.steps.last();
    println!(
        "{} {}",
        bold("Exit Code:"),
        last_step
            .and_then(|s| s.exit_code)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Error Code:"),
        last_step
            .and_then(|s| s.error_code.as_deref())
            .unwrap_or("-")
    );
    println!(
        "{} {}",
        bold("Error Message:"),
        last_step
            .and_then(|s| s.error_message.as_deref())
            .map(|value| value.replace('\n', " "))
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Created:"),
        dimmed(&run.created_at.to_rfc3339())
    );
}
