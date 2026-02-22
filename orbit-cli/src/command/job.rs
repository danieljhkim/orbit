use clap::{Args, Subcommand};
use orbit_core::command::job::JobAddParams;
use orbit_core::{Job, JobSession, OrbitError, OrbitRuntime};
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
    Pause(JobPauseArgs),
    Resume(JobResumeArgs),
    Cancel(JobCancelArgs),
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
            JobSubcommand::Pause(args) => args.execute(runtime),
            JobSubcommand::Resume(args) => args.execute(runtime),
            JobSubcommand::Cancel(args) => args.execute(runtime),
            JobSubcommand::History(args) => args.execute(runtime),
            JobSubcommand::Delete(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct JobAddArgs {
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub schedule: String,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub timezone: Option<String>,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let job = runtime.add_job(JobAddParams {
            name: self.name,
            task_id: self.task,
            schedule_spec: self.schedule,
            timezone: self.timezone,
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
                "{:<26} {:<8} {:<26} {:<20} NAME",
                "JOB_ID", "STATE", "TASK_ID", "NEXT_RUN_AT"
            );
            for job in &jobs {
                println!(
                    "{:<26} {:<8} {:<26} {:<20} {}",
                    job.job_id,
                    job.state,
                    job.task_id,
                    job.next_run_at
                        .map(|v| v.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
                    job.name
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
            println!("Job ID:       {}", job.job_id);
            println!("Name:         {}", job.name);
            println!("Task:         {}", job.task_id);
            println!("Schedule:     {}", job.schedule_spec);
            println!("Timezone:     {}", job.timezone);
            println!("State:        {}", job.state);
            println!(
                "Next run:     {}",
                job.next_run_at
                    .map(|v| v.to_rfc3339())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "Last run:     {}",
                job.last_run_at
                    .map(|v| v.to_rfc3339())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "Last session: {}",
                job.last_run_session_id.unwrap_or_else(|| "-".to_string())
            );
            if let Some(ref err) = job.last_error {
                println!("Last error:   {err}");
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobRunArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime.run_job_now(&self.job_id)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "job_id": run.job_id,
                "session_id": run.session_id,
                "status": run.status.to_string()
            }))
        } else {
            println!(
                "job_id={};session_id={};status={}",
                run.job_id, run.session_id, run.status
            );
            Ok(())
        }
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
pub struct JobCancelArgs {
    pub job_id: String,
}

impl Execute for JobCancelArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let session_id = runtime.cancel_job(&self.job_id)?;
        println!(
            "Cancellation requested for job '{}' session '{}'",
            self.job_id, session_id
        );
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
        let sessions = runtime.job_history(&self.job_id)?;
        if self.json {
            let values = sessions.iter().map(job_session_to_json).collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            println!(
                "{:<30} {:<10} {:<12} {:<26} {:<26}",
                "SESSION_ID", "TRIGGER", "STATUS", "STARTED_AT", "FINISHED_AT"
            );
            for session in &sessions {
                println!(
                    "{:<30} {:<10} {:<12} {:<26} {:<26}",
                    session.session_id,
                    session.trigger,
                    session.status,
                    session
                        .started_at
                        .map(|v| v.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
                    session
                        .finished_at
                        .map(|v| v.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string()),
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

fn job_to_json(job: &Job) -> Value {
    json!({
        "job_id": job.job_id,
        "name": job.name,
        "task_id": job.task_id,
        "schedule_spec": job.schedule_spec,
        "timezone": job.timezone,
        "state": job.state.to_string(),
        "created_at": job.created_at.to_rfc3339(),
        "updated_at": job.updated_at.to_rfc3339(),
        "paused_at": job.paused_at.map(|v| v.to_rfc3339()),
        "deleted_at": job.deleted_at.map(|v| v.to_rfc3339()),
        "last_run_session_id": job.last_run_session_id,
        "last_run_at": job.last_run_at.map(|v| v.to_rfc3339()),
        "next_run_at": job.next_run_at.map(|v| v.to_rfc3339()),
        "last_error": job.last_error
    })
}

fn job_session_to_json(session: &JobSession) -> Value {
    json!({
        "session_id": session.session_id,
        "job_id": session.job_id,
        "task_id": session.task_id,
        "trigger": session.trigger.to_string(),
        "trigger_time": session.trigger_time.to_rfc3339(),
        "started_at": session.started_at.map(|v| v.to_rfc3339()),
        "finished_at": session.finished_at.map(|v| v.to_rfc3339()),
        "status": session.status.to_string(),
        "exit_code": session.exit_code,
        "error": session.error,
        "composed_context_hash": session.composed_context_hash,
        "effective_allowlist_hash": session.effective_allowlist_hash,
        "created_by_role": session.created_by_role.to_string(),
        "created_at": session.created_at.to_rfc3339(),
        "cancel_requested_at": session.cancel_requested_at.map(|v| v.to_rfc3339()),
    })
}
