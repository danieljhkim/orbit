use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use clap::{Args, Subcommand};
use orbit_core::command::job::JobAddParams;
use orbit_core::job::runtime::{JobRuntime, JobRuntimeConfig, ShutdownSignal};
use orbit_core::{Job, JobRetryBackoffStrategy, JobRun, JobTargetType, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct JobCommand {
    #[command(subcommand)]
    pub command: JobSubcommand,
}

impl Execute for JobCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum JobSubcommand {
    Add(JobAddArgs),
    List(JobListArgs),
    Show(JobShowArgs),
    Run(JobRunArgs),
    Tick(JobTickArgs),
    Serve(JobServeArgs),
    Pause(JobPauseArgs),
    Resume(JobResumeArgs),
    History(JobHistoryArgs),
    Delete(JobDeleteArgs),
    Archive(JobArchiveRunArgs),
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::Add(args) => args.execute(runtime),
            JobSubcommand::List(args) => args.execute(runtime),
            JobSubcommand::Show(args) => args.execute(runtime),
            JobSubcommand::Run(args) => args.execute(runtime),
            JobSubcommand::Tick(args) => args.execute(runtime),
            JobSubcommand::Serve(args) => args.execute(runtime),
            JobSubcommand::Pause(args) => args.execute(runtime),
            JobSubcommand::Resume(args) => args.execute(runtime),
            JobSubcommand::History(args) => args.execute(runtime),
            JobSubcommand::Delete(args) => args.execute(runtime),
            JobSubcommand::Archive(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct JobAddArgs {
    #[arg(long)]
    pub target_id: String,
    #[arg(long)]
    pub schedule: String,
    #[arg(long)]
    pub agent_cli: String,
    #[arg(long, default_value = "15m")]
    pub timeout: String,
    #[arg(long, default_value_t = 0)]
    pub retry_max_attempts: u32,
    #[arg(long, value_enum, default_value_t = JobRetryBackoffStrategy::None)]
    pub retry_backoff: JobRetryBackoffStrategy,
    #[arg(long, default_value = "0s")]
    pub retry_initial_delay: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let timeout_seconds = parse_duration_seconds(&self.timeout)?;
        let retry_initial_delay_seconds = parse_duration_seconds(&self.retry_initial_delay)?;

        let job = runtime.add_job(JobAddParams {
            target_type: JobTargetType::Activity,
            target_id: self.target_id,
            schedule: self.schedule,
            agent_cli: self.agent_cli,
            timeout_seconds,
            retry_max_attempts: self.retry_max_attempts,
            retry_backoff_strategy: self.retry_backoff,
            retry_initial_delay_seconds,
        })?;

        if self.json {
            crate::output::json::print_pretty(&job_to_json(&job))
        } else {
            println!("{}", job.job_id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobListArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let jobs = runtime.list_jobs(self.all)?;
        if self.json {
            let values = jobs.iter().map(job_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!(
                "{:<26} {:<15} {:<28} {:<9} {:<20}",
                "JOB_ID", "TARGET_TYPE", "TARGET_ID", "STATE", "NEXT_RUN_AT"
            );
            for job in &jobs {
                println!(
                    "{:<26} {:<15} {:<28} {:<9} {:<20}",
                    job.job_id,
                    job.target_type,
                    job.target_id,
                    job.state,
                    job.next_run_at.to_rfc3339(),
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobShowArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let job = runtime.show_job(&self.job_id)?;
        if self.json {
            crate::output::json::print_pretty(&job_to_json(&job))
        } else {
            println!("Job ID:              {}", job.job_id);
            println!("Target Type:         {}", job.target_type);
            println!("Target ID:           {}", job.target_id);
            println!("Schedule:            {}", job.schedule);
            println!("Agent CLI:           {}", job.agent_cli);
            println!("Timeout (seconds):   {}", job.timeout_seconds);
            println!("Retry Max Attempts:  {}", job.retry_max_attempts);
            println!("Retry Backoff:       {}", job.retry_backoff_strategy);
            println!("Retry Initial Delay: {}", job.retry_initial_delay_seconds);
            println!("State:               {}", job.state);
            println!("Next Run:            {}", job.next_run_at.to_rfc3339());
            println!("Created:             {}", job.created_at.to_rfc3339());
            println!("Updated:             {}", job.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunArgs {
    pub job_id: String,
    #[arg(long)]
    pub task_id: Option<String>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime
            .run_job_now_with_input(&self.job_id, build_job_run_input(self.task_id.as_deref())?)?;
        let run_details = runtime
            .job_history(&self.job_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);
        if self.json {
            crate::output::json::print_pretty(&json!({
                "job_id": run.job_id,
                "run_id": run.run_id,
                "state": run.state.to_string(),
                "attempt": run.attempt,
                "error_code": run_details.as_ref().and_then(|entry| entry.error_code.clone()),
                "error_message": run_details.as_ref().and_then(|entry| entry.error_message.clone()),
            }))
        } else {
            let error_code = run_details
                .as_ref()
                .and_then(|entry| entry.error_code.clone())
                .unwrap_or_else(|| "-".to_string());
            let error_message = run_details
                .as_ref()
                .and_then(|entry| entry.error_message.clone())
                .unwrap_or_else(|| "-".to_string())
                .replace('\n', " ");
            println!(
                "job_id={};run_id={};state={};attempt={};error_code={};error_message={}",
                run.job_id, run.run_id, run.state, run.attempt, error_code, error_message
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobTickArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobTickArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tick = JobRuntime::new(runtime, JobRuntimeConfig::default()).tick_once(Utc::now())?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "ran": tick.ran,
                "next_wake_at": tick.next_wake_at.map(|value| value.to_rfc3339()),
            }))
        } else {
            println!(
                "ran={};next_wake_at={}",
                tick.ran,
                tick.next_wake_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| "-".to_string())
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobServeArgs {
    #[arg(long, default_value = "30s")]
    pub idle_sleep: String,
    #[arg(long, default_value = "5m")]
    pub max_sleep: String,
}

impl Execute for JobServeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let shutdown = CliShutdownSignal::install()?;
        let job_runtime = JobRuntime::new(
            runtime,
            JobRuntimeConfig {
                idle_sleep: Duration::from_secs(parse_duration_seconds(&self.idle_sleep)?),
                max_sleep: Duration::from_secs(parse_duration_seconds(&self.max_sleep)?),
            },
        );
        job_runtime.run_forever(&shutdown)
    }
}

#[derive(Args)]
pub struct JobPauseArgs {
    pub job_id: String,
}

impl Execute for JobPauseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.pause_job(&self.job_id)?;
        println!("Paused job '{}'", self.job_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct JobResumeArgs {
    pub job_id: String,
}

impl Execute for JobResumeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.resume_job(&self.job_id)?;
        println!("Resumed job '{}'", self.job_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct JobHistoryArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = runtime.job_history(&self.job_id)?;
        if self.json {
            let values = runs.iter().map(job_run_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!(
                "{:<30} {:<7} {:<10} {:<26} {:<26} {:<24} ERROR_MESSAGE",
                "RUN_ID", "ATTEMPT", "STATE", "STARTED_AT", "FINISHED_AT", "ERROR_CODE"
            );
            for run in &runs {
                println!(
                    "{:<30} {:<7} {:<10} {:<26} {:<26} {:<24} {}",
                    run.run_id,
                    run.attempt,
                    run.state,
                    run.started_at
                        .map(|v| v.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
                    run.finished_at
                        .map(|v| v.to_rfc3339())
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
pub struct JobDeleteArgs {
    pub job_id: String,
}

impl Execute for JobDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_job(&self.job_id)?;
        println!("Deleted job '{}'", self.job_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct JobArchiveRunArgs {
    pub job_run_id: String,
}

impl Execute for JobArchiveRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.archive_job_run(&self.job_run_id)?;
        println!("Archived job run '{}'", self.job_run_id);
        Ok(())
    }
}

fn job_to_json(job: &Job) -> Value {
    json!({
        "job_id": job.job_id,
        "target_type": job.target_type.to_string(),
        "target_id": job.target_id,
        "schedule": job.schedule,
        "agent_cli": job.agent_cli,
        "timeout_seconds": job.timeout_seconds,
        "retry_max_attempts": job.retry_max_attempts,
        "retry_backoff_strategy": job.retry_backoff_strategy.to_string(),
        "retry_initial_delay_seconds": job.retry_initial_delay_seconds,
        "state": job.state.to_string(),
        "next_run_at": job.next_run_at.to_rfc3339(),
        "created_at": job.created_at.to_rfc3339(),
        "updated_at": job.updated_at.to_rfc3339(),
    })
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|v| v.to_rfc3339()),
        "finished_at": run.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "exit_code": run.exit_code,
        "agent_response_json": run.agent_response_json,
        "error_code": run.error_code,
        "error_message": run.error_message,
        "created_at": run.created_at.to_rfc3339(),
    })
}

pub(crate) fn parse_duration_seconds(raw: &str) -> Result<u64, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(
            "duration must not be empty".to_string(),
        ));
    }

    let split_at = value
        .find(|c: char| c.is_alphabetic())
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid duration: {raw}")))?;
    let (num_raw, unit_raw) = value.split_at(split_at);

    let num: u64 = num_raw
        .parse()
        .map_err(|_| OrbitError::InvalidInput(format!("invalid duration number: {raw}")))?;

    let seconds = match unit_raw {
        "s" => num,
        "m" => num.saturating_mul(60),
        "h" => num.saturating_mul(3600),
        "d" => num.saturating_mul(86400),
        "w" => num.saturating_mul(604800),
        _ => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid duration unit: {unit_raw} (expected s/m/h/d/w)"
            )));
        }
    };

    Ok(seconds)
}

pub(crate) fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}

fn build_job_run_input(task_id: Option<&str>) -> Result<Value, OrbitError> {
    match task_id {
        None => Ok(json!({})),
        Some(task_id) => {
            let task_id = task_id.trim();
            if task_id.is_empty() {
                return Err(OrbitError::InvalidInput(
                    "task_id must not be empty".to_string(),
                ));
            }
            Ok(json!({ "task_id": task_id }))
        }
    }
}

struct CliShutdownSignal;

impl CliShutdownSignal {
    fn install() -> Result<Self, OrbitError> {
        reset_shutdown_signal();
        install_shutdown_handlers()?;
        Ok(Self)
    }
}

impl ShutdownSignal for CliShutdownSignal {
    fn should_stop(&self) -> bool {
        shutdown_requested()
    }
}

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

fn reset_shutdown_signal() {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
}

fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(unix)]
fn install_shutdown_handlers() -> Result<(), OrbitError> {
    unsafe extern "C" fn handle_signal(_: i32) {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    }

    unsafe extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }

    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;

    unsafe {
        signal(SIGINT, handle_signal as *const () as usize);
        signal(SIGTERM, handle_signal as *const () as usize);
    }

    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_handlers() -> Result<(), OrbitError> {
    Ok(())
}
