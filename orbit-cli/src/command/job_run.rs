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
            println!(
                "{:<30} {:<26} {:<7} {:<10} {:<26} {:<26} {:<24} ERROR_MESSAGE",
                "RUN_ID", "JOB_ID", "ATTEMPT", "STATE", "STARTED_AT", "FINISHED_AT", "ERROR_CODE"
            );
            for run in &runs {
                println!(
                    "{:<30} {:<26} {:<7} {:<10} {:<26} {:<26} {:<24} {}",
                    run.run_id,
                    run.job_id,
                    run.attempt,
                    run.state,
                    run.started_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
                    run.finished_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
                    run.error_code.clone().unwrap_or_else(|| "-".to_string()),
                    summarize_error_message(run.error_message.as_deref()),
                );
            }
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
    println!("Run ID:              {}", run.run_id);
    println!("Job ID:              {}", run.job_id);
    println!("Attempt:             {}", run.attempt);
    println!("State:               {}", run.state);
    println!("Scheduled:           {}", run.scheduled_at.to_rfc3339());
    println!(
        "Started:             {}",
        run.started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Finished:            {}",
        run.finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Duration (ms):       {}",
        run.duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Exit Code:           {}",
        run.exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Error Code:          {}",
        run.error_code.as_deref().unwrap_or("-")
    );
    println!(
        "Error Message:       {}",
        run.error_message
            .as_deref()
            .map(|value| value.replace('\n', " "))
            .unwrap_or_else(|| "-".to_string())
    );
    println!("Created:             {}", run.created_at.to_rfc3339());
}
