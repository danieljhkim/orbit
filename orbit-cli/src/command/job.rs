use clap::{Args, Subcommand};
use orbit_core::command::job::JobAddParams;
use orbit_core::{Job, JobRun, JobStep, JobTargetType, OrbitError, OrbitRuntime};
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
    History(JobHistoryArgs),
    Delete(JobDeleteArgs),
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::Add(args) => args.execute(runtime),
            JobSubcommand::List(args) => args.execute(runtime),
            JobSubcommand::Show(args) => args.execute(runtime),
            JobSubcommand::Run(args) => args.execute(runtime),
            JobSubcommand::History(args) => args.execute(runtime),
            JobSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct JobAddArgs {
    #[arg(long)]
    pub job_id: Option<String>,
    #[arg(long, default_value_t = 1)]
    pub max_active_runs: u32,
    #[arg(long)]
    pub target_id: String,
    #[arg(long)]
    pub agent_cli: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, default_value = "20m")]
    pub timeout: String,
    /// Comma-separated list of extra env var names to pass through in hermetic mode for this job.
    #[arg(long, default_value = "")]
    pub env_extra: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let timeout_seconds = crate::parse::parse_duration_seconds(&self.timeout)?;

        let job = runtime.add_job(JobAddParams {
            job_id: self.job_id,
            default_input: None,
            max_active_runs: Some(self.max_active_runs),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: self.target_id,
                agent_cli: self.agent_cli,
                model: self.model,
                timeout_seconds,
                env_extra: crate::parse::csv_to_vec(&self.env_extra),
                retry_max_attempts: 0,
                retry_backoff_seconds: 10,
            }],
            initial_state_override: None,
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
    /// Output signal-tier JSON (job_id, target_id, state only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for JobListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.ops {
            let jobs = runtime.list_jobs(self.all)?;
            let values = jobs.iter().map(job_to_signal_json).collect::<Vec<_>>();
            return crate::output::json::print_pretty(&Value::Array(values));
        }

        let jobs_with_runs = runtime.list_jobs_with_last_run(self.all)?;
        if self.json {
            let values = jobs_with_runs
                .iter()
                .map(|(job, last_run)| job_to_json_with_last_run(job, last_run.as_ref()))
                .collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!(
                "{:<26} {:<15} {:<28} {:<16} {:<9} LAST_RUN",
                "JOB_ID", "TARGET_TYPE", "TARGET_ID", "MODEL", "STATE"
            );
            for (job, last_run) in &jobs_with_runs {
                let first = job.steps.first();
                println!(
                    "{:<26} {:<15} {:<28} {:<16} {:<9} {}",
                    job.job_id,
                    first.map(|s| s.target_type.to_string()).unwrap_or_default(),
                    first.map(|s| s.target_id.as_str()).unwrap_or("-"),
                    first.and_then(|s| s.model.as_deref()).unwrap_or("-"),
                    job.state,
                    format_last_run(last_run.as_ref()),
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
            println!("Job ID:            {}", job.job_id);
            println!("State:             {}", job.state);
            println!("Max Active Runs:   {}", job.max_active_runs);
            if let Some(default_input) = &job.default_input {
                let rendered = serde_json::to_string(default_input)
                    .unwrap_or_else(|_| "<invalid-json>".to_string());
                println!("Default Input:     {}", rendered);
            }
            println!("Steps:             {}", job.steps.len());
            for (i, step) in job.steps.iter().enumerate() {
                println!("  Step {}:", i + 1);
                println!("    Target Type:    {}", step.target_type);
                println!("    Target ID:      {}", step.target_id);
                println!("    Agent CLI:      {}", step.agent_cli);
                if let Some(model) = &step.model {
                    println!("    Model:          {}", model);
                }
                println!("    Timeout (s):    {}", step.timeout_seconds);
            }
            println!("Created:           {}", job.created_at.to_rfc3339());
            println!("Updated:           {}", job.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunArgs {
    pub job_id: String,
    /// Input key=value pairs passed to all job steps (repeatable).
    /// Example: --input task_id=T123 --input base=main
    #[arg(long)]
    pub input: Vec<String>,
    #[arg(long)]
    pub json: bool,
    /// Stream agent stderr to the terminal and tee stdout live for debugging.
    #[arg(long)]
    pub debug: bool,
}

impl Execute for JobRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime.run_job_now_with_input_debug(
            &self.job_id,
            build_job_run_input(&self.input)?,
            self.debug,
        )?;
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
                "error_code": run_details.as_ref().and_then(|entry| entry.steps.last()).and_then(|s| s.error_code.clone()),
                "error_message": run_details.as_ref().and_then(|entry| entry.steps.last()).and_then(|s| s.error_message.clone()),
            }))
        } else {
            let error_code = run_details
                .as_ref()
                .and_then(|entry| entry.steps.last())
                .and_then(|s| s.error_code.clone())
                .unwrap_or_else(|| "-".to_string());
            let error_message = run_details
                .as_ref()
                .and_then(|entry| entry.steps.last())
                .and_then(|s| s.error_message.clone())
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
                    run.steps
                        .last()
                        .and_then(|s| s.error_code.clone())
                        .unwrap_or_else(|| "-".to_string()),
                    summarize_error_message(
                        run.steps.last().and_then(|s| s.error_message.as_deref())
                    ),
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobDeleteArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_job(&self.job_id)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": self.job_id,
                "deleted": true,
            }))
        } else {
            println!("Deleted job '{}'", self.job_id);
            Ok(())
        }
    }
}

fn format_last_run(last_run: Option<&JobRun>) -> String {
    match last_run {
        None => "never".to_string(),
        Some(run) => {
            let ts = run
                .finished_at
                .or(run.started_at)
                .unwrap_or(run.scheduled_at);
            format!("{} {}", run.state, ts.format("%Y-%m-%dT%H:%M:%SZ"))
        }
    }
}

fn job_to_json_with_last_run(job: &Job, last_run: Option<&JobRun>) -> Value {
    let mut obj = job_to_json(job);
    obj["last_run_state"] = last_run
        .map(|r| serde_json::Value::String(r.state.to_string()))
        .unwrap_or(serde_json::Value::Null);
    obj["last_run_at"] = last_run
        .and_then(|r| r.finished_at.or(r.started_at).or(Some(r.scheduled_at)))
        .map(|ts| serde_json::Value::String(ts.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);
    obj
}

fn job_to_signal_json(job: &Job) -> Value {
    let first = job.steps.first();
    json!({
        "job_id": job.job_id,
        "target_id": first.map(|s| s.target_id.as_str()).unwrap_or(""),
        "state": job.state.to_string(),
    })
}

fn job_to_json(job: &Job) -> Value {
    json!({
        "job_id": job.job_id,
        "state": job.state.to_string(),
        "default_input": job.default_input,
        "max_active_runs": job.max_active_runs,
        "created_at": job.created_at.to_rfc3339(),
        "updated_at": job.updated_at.to_rfc3339(),
        "steps": job.steps.iter().map(job_step_to_json).collect::<Vec<_>>(),
    })
}

fn job_step_to_json(step: &JobStep) -> Value {
    let mut value = json!({
        "target_type": step.target_type.to_string(),
        "target_id": step.target_id,
        "agent_cli": step.agent_cli,
        "timeout_seconds": step.timeout_seconds,
        "env_extra": step.env_extra,
    });
    if let Some(model) = &step.model {
        value["model"] = Value::String(model.clone());
    }
    value
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    let last = run.steps.last();
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|v| v.to_rfc3339()),
        "finished_at": run.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "exit_code": last.and_then(|s| s.exit_code),
        "agent_response_json": last.and_then(|s| s.agent_response_json.as_ref()),
        "error_code": last.and_then(|s| s.error_code.as_deref()),
        "error_message": last.and_then(|s| s.error_message.as_deref()),
        "steps": run.steps.iter().map(|s| json!({
            "step_index": s.step_index,
            "target_type": s.target_type.to_string(),
            "target_id": s.target_id,
            "state": s.state.to_string(),
            "started_at": s.started_at.map(|v| v.to_rfc3339()),
            "finished_at": s.finished_at.map(|v| v.to_rfc3339()),
            "duration_ms": s.duration_ms,
            "exit_code": s.exit_code,
            "agent_response_json": s.agent_response_json,
            "error_code": s.error_code,
            "error_message": s.error_message,
        })).collect::<Vec<_>>(),
        "created_at": run.created_at.to_rfc3339(),
    })
}

pub(crate) fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}

fn build_job_run_input(pairs: &[String]) -> Result<Value, OrbitError> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "invalid --input entry \"{pair}\": expected key=value"
            ))
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "invalid --input entry \"{pair}\": key must not be empty"
            )));
        }
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    Ok(Value::Object(map))
}
