use clap::{Args, Subcommand};
use orbit_core::command::scheduler::SchedulerAddParams;
use orbit_core::{
    OrbitError, OrbitRuntime, Scheduler, SchedulerRetryBackoffStrategy, SchedulerRun,
    SchedulerTargetType,
};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct SchedulerCommand {
    #[command(subcommand)]
    pub command: SchedulerSubcommand,
}

impl Execute for SchedulerCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum SchedulerSubcommand {
    Add(SchedulerAddArgs),
    List(SchedulerListArgs),
    Show(SchedulerShowArgs),
    Run(SchedulerRunArgs),
    Pause(SchedulerPauseArgs),
    Resume(SchedulerResumeArgs),
    History(SchedulerHistoryArgs),
    Delete(SchedulerDeleteArgs),
}

impl Execute for SchedulerSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            SchedulerSubcommand::Add(args) => args.execute(runtime),
            SchedulerSubcommand::List(args) => args.execute(runtime),
            SchedulerSubcommand::Show(args) => args.execute(runtime),
            SchedulerSubcommand::Run(args) => args.execute(runtime),
            SchedulerSubcommand::Pause(args) => args.execute(runtime),
            SchedulerSubcommand::Resume(args) => args.execute(runtime),
            SchedulerSubcommand::History(args) => args.execute(runtime),
            SchedulerSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct SchedulerAddArgs {
    #[arg(long)]
    pub target_id: String,
    #[arg(long)]
    pub schedule: String,
    #[arg(long)]
    pub agent_cli: String,
    #[arg(long, default_value = "5m")]
    pub timeout: String,
    #[arg(long, default_value_t = 0)]
    pub retry_max_attempts: u32,
    #[arg(long, value_enum, default_value_t = SchedulerRetryBackoffStrategy::None)]
    pub retry_backoff: SchedulerRetryBackoffStrategy,
    #[arg(long, default_value = "0s")]
    pub retry_initial_delay: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SchedulerAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let timeout_seconds = parse_duration_seconds(&self.timeout)?;
        let retry_initial_delay_seconds = parse_duration_seconds(&self.retry_initial_delay)?;

        let scheduler = runtime.add_scheduler(SchedulerAddParams {
            target_type: SchedulerTargetType::Job,
            target_id: self.target_id,
            schedule: self.schedule,
            agent_cli: self.agent_cli,
            timeout_seconds,
            retry_max_attempts: self.retry_max_attempts,
            retry_backoff_strategy: self.retry_backoff,
            retry_initial_delay_seconds,
        })?;

        if self.json {
            crate::output::json::print_pretty(&scheduler_to_json(&scheduler))
        } else {
            println!("{}", scheduler.scheduler_id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct SchedulerListArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SchedulerListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let schedulers = runtime.list_schedulers(self.all)?;
        if self.json {
            let values = schedulers.iter().map(scheduler_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!(
                "{:<26} {:<15} {:<28} {:<9} {:<20}",
                "JOB_ID", "TARGET_TYPE", "TARGET_ID", "STATE", "NEXT_RUN_AT"
            );
            for scheduler in &schedulers {
                println!(
                    "{:<26} {:<15} {:<28} {:<9} {:<20}",
                    scheduler.scheduler_id,
                    scheduler.target_type,
                    scheduler.target_id,
                    scheduler.state,
                    scheduler.next_run_at.to_rfc3339(),
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct SchedulerShowArgs {
    pub scheduler_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SchedulerShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let scheduler = runtime.show_scheduler(&self.scheduler_id)?;
        if self.json {
            crate::output::json::print_pretty(&scheduler_to_json(&scheduler))
        } else {
            println!("Scheduler ID:              {}", scheduler.scheduler_id);
            println!("Target Type:         {}", scheduler.target_type);
            println!("Target ID:           {}", scheduler.target_id);
            println!("Schedule:            {}", scheduler.schedule);
            println!("Agent CLI:           {}", scheduler.agent_cli);
            println!("Timeout (seconds):   {}", scheduler.timeout_seconds);
            println!("Retry Max Attempts:  {}", scheduler.retry_max_attempts);
            println!("Retry Backoff:       {}", scheduler.retry_backoff_strategy);
            println!(
                "Retry Initial Delay: {}",
                scheduler.retry_initial_delay_seconds
            );
            println!("State:               {}", scheduler.state);
            println!(
                "Next Run:            {}",
                scheduler.next_run_at.to_rfc3339()
            );
            println!("Created:             {}", scheduler.created_at.to_rfc3339());
            println!("Updated:             {}", scheduler.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct SchedulerRunArgs {
    pub scheduler_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SchedulerRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime.run_scheduler_now(&self.scheduler_id)?;
        let run_details = runtime
            .scheduler_history(&self.scheduler_id)?
            .into_iter()
            .find(|entry| entry.run_id == run.run_id);
        if self.json {
            crate::output::json::print_pretty(&json!({
                "scheduler_id": run.scheduler_id,
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
                "scheduler_id={};run_id={};state={};attempt={};error_code={};error_message={}",
                run.scheduler_id, run.run_id, run.state, run.attempt, error_code, error_message
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct SchedulerPauseArgs {
    pub scheduler_id: String,
}

impl Execute for SchedulerPauseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.pause_scheduler(&self.scheduler_id)?;
        println!("Paused scheduler '{}'", self.scheduler_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct SchedulerResumeArgs {
    pub scheduler_id: String,
}

impl Execute for SchedulerResumeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.resume_scheduler(&self.scheduler_id)?;
        println!("Resumed scheduler '{}'", self.scheduler_id);
        Ok(())
    }
}

#[derive(Args)]
pub struct SchedulerHistoryArgs {
    pub scheduler_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for SchedulerHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let runs = runtime.scheduler_history(&self.scheduler_id)?;
        if self.json {
            let values = runs.iter().map(scheduler_run_to_json).collect::<Vec<_>>();
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
pub struct SchedulerDeleteArgs {
    pub scheduler_id: String,
}

impl Execute for SchedulerDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_scheduler(&self.scheduler_id)?;
        println!("Deleted scheduler '{}'", self.scheduler_id);
        Ok(())
    }
}

fn scheduler_to_json(scheduler: &Scheduler) -> Value {
    json!({
        "scheduler_id": scheduler.scheduler_id,
        "target_type": scheduler.target_type.to_string(),
        "target_id": scheduler.target_id,
        "schedule": scheduler.schedule,
        "agent_cli": scheduler.agent_cli,
        "timeout_seconds": scheduler.timeout_seconds,
        "retry_max_attempts": scheduler.retry_max_attempts,
        "retry_backoff_strategy": scheduler.retry_backoff_strategy.to_string(),
        "retry_initial_delay_seconds": scheduler.retry_initial_delay_seconds,
        "state": scheduler.state.to_string(),
        "next_run_at": scheduler.next_run_at.to_rfc3339(),
        "created_at": scheduler.created_at.to_rfc3339(),
        "updated_at": scheduler.updated_at.to_rfc3339(),
    })
}

fn scheduler_run_to_json(run: &SchedulerRun) -> Value {
    json!({
        "run_id": run.run_id,
        "scheduler_id": run.scheduler_id,
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

fn parse_duration_seconds(raw: &str) -> Result<u64, OrbitError> {
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

fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}
