use chrono::{Duration, Utc};
use clap::{Args, Subcommand};
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::{JobRun, JobRunState, JobRunStep, OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;
use crate::command::job::{job_run_to_json, summarize_error_message};

#[derive(Args)]
#[command(about = "Inspect and manage job run history")]
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
    /// List job runs with optional filters
    List(JobRunListArgs),
    /// Show details of a specific run
    Show(JobRunShowArgs),
    /// Cancel a running or scheduled run
    Cancel(JobRunCancelArgs),
    /// Archive a completed run
    Archive(JobRunArchiveArgs),
    /// Delete a run record
    Delete(JobRunDeleteArgs),
    /// Retry a failed run from a specific step
    Retry(JobRunRetryArgs),
}

impl Execute for JobRunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobRunSubcommand::List(args) => args.execute(runtime),
            JobRunSubcommand::Show(args) => args.execute(runtime),
            JobRunSubcommand::Cancel(args) => args.execute(runtime),
            JobRunSubcommand::Archive(args) => args.execute(runtime),
            JobRunSubcommand::Delete(args) => args.execute(runtime),
            JobRunSubcommand::Retry(args) => args.execute(runtime),
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
                    Cell::new(run.attempt.to_string()),
                    crate::output::color::job_state_color_cell(&run.state.to_string()),
                    Cell::new(
                        run.started_at
                            .map(|value| value.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(
                        run.finished_at
                            .map(|value| value.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(
                        run.steps
                            .last()
                            .and_then(|s| s.error_code.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    Cell::new(summarize_error_message(
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
    /// Dump full details for a specific step (0-based index).
    #[arg(long)]
    pub step: Option<usize>,
}

impl Execute for JobRunShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime.show_job_run(&self.run_id)?;
        if let Some(step_index) = self.step {
            let step = run
                .steps
                .iter()
                .find(|s| s.step_index as usize == step_index)
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "step {step_index} not found in run '{}' (run has {} step(s))",
                        self.run_id,
                        run.steps.len()
                    ))
                })?;
            if self.json {
                use serde_json::json;
                crate::output::json::print_pretty(&json!({
                    "step_index": step.step_index,
                    "target_type": step.target_type.to_string(),
                    "target_id": step.target_id,
                    "state": step.state.to_string(),
                    "started_at": step.started_at.map(|v| v.to_rfc3339()),
                    "finished_at": step.finished_at.map(|v| v.to_rfc3339()),
                    "duration_ms": step.duration_ms,
                    "exit_code": step.exit_code,
                    "error_code": step.error_code,
                    "error_message": step.error_message,
                    "agent_response_json": step.agent_response_json,
                }))
            } else {
                print_step_detail(step);
                Ok(())
            }
        } else if self.json {
            crate::output::json::print_pretty(&job_run_to_json(&run))
        } else {
            print_job_run(&run);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunCancelArgs {
    pub run_id: String,
}

impl Execute for JobRunCancelArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.cancel_job_run(&self.run_id)?;
        println!("Cancelled job run '{}'", self.run_id);
        Ok(())
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

#[derive(Args)]
pub struct JobRunRetryArgs {
    /// The run ID of the failed run to retry from.
    pub run_id: String,
    /// The target_id of the step to resume from.
    #[arg(long)]
    pub step: String,
    /// Stream agent stderr to the terminal.
    #[arg(long)]
    pub debug: bool,
    /// Output result as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobRunRetryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.retry_job_run(&self.run_id, &self.step, self.debug)?;
        if self.json {
            use serde_json::json;
            crate::output::json::print_pretty(&json!({
                "run_id": result.run_id,
                "job_id": result.job_id,
                "state": result.state.to_string(),
                "attempt": result.attempt,
            }))
        } else {
            println!(
                "Retry run '{}' for job '{}' completed with state: {}",
                result.run_id, result.job_id, result.state
            );
            Ok(())
        }
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
    println!(
        "{} {}",
        bold("Created:"),
        dimmed(&run.created_at.to_rfc3339())
    );

    if run.steps.is_empty() {
        println!("\n{}", bold("Steps: (none)"));
    } else {
        println!("\n{}", bold("Steps:"));
        let mut table = crate::output::table::build_table(&[
            "STEP",
            "TARGET_ID",
            "STATE",
            "DURATION_MS",
            "EXIT_CODE",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]);
        for step in &run.steps {
            use comfy_table::Cell;
            table.add_row(vec![
                Cell::new(step.step_index.to_string()),
                Cell::new(&step.target_id),
                crate::output::color::job_state_color_cell(&step.state.to_string()),
                Cell::new(
                    step.duration_ms
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::new(
                    step.exit_code
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::new(step.error_code.as_deref().unwrap_or("-")),
                Cell::new(summarize_error_message(step.error_message.as_deref())),
            ]);
        }
        println!("{table}");
        println!(
            "  {}",
            dimmed("Use --step <n> to inspect full details for a step.")
        );
    }
}

fn print_step_detail(step: &JobRunStep) {
    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Step Index:"), step.step_index);
    println!("{} {}", bold("Target Type:"), step.target_type);
    println!("{} {}", bold("Target ID:"), step.target_id);
    println!(
        "{} {}",
        bold("State:"),
        job_state_color(&step.state.to_string())
    );
    println!(
        "{} {}",
        bold("Started:"),
        step.started_at
            .map(|v| v.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Finished:"),
        step.finished_at
            .map(|v| v.to_rfc3339())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Duration (ms):"),
        step.duration_ms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Exit Code:"),
        step.exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Error Code:"),
        step.error_code.as_deref().unwrap_or("-")
    );
    println!(
        "{} {}",
        bold("Error Message:"),
        step.error_message.as_deref().unwrap_or("-")
    );
    if let Some(resp) = &step.agent_response_json {
        let rendered =
            serde_json::to_string_pretty(resp).unwrap_or_else(|_| "<invalid-json>".to_string());
        println!("{}", bold("Agent Response:"));
        for line in rendered.lines() {
            println!("  {}", dimmed(line));
        }
    } else {
        println!("{} -", bold("Agent Response:"));
    }
}
