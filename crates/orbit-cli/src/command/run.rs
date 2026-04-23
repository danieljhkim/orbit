use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::{duel, job, ship};

const RUN_AFTER_HELP: &str = "\
Workflow entrypoints:
  orbit run ship <task_id> ...
  orbit run duel [score|list|show|plan]
  orbit run job <job_id> [--input key=value] [--json] [--debug]

Direct form:
  orbit run <job_id> [--input key=value] [--json] [--debug]
    Equivalent to `orbit run job <job_id>`.
";

#[derive(Args)]
#[command(
    about = "Run a job workflow (supports run ship / run duel / run job / run <id>)",
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true,
    override_usage = "orbit run <COMMAND>\n       orbit run <JOB_ID> [OPTIONS]",
    after_help = RUN_AFTER_HELP
)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: Option<RunSubcommand>,

    #[command(flatten)]
    pub positional: PositionalJobArgs,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            Some(command) => command.execute(runtime),
            None => execute_positional_job(self.positional, runtime),
        }
    }
}

#[derive(Subcommand)]
pub enum RunSubcommand {
    /// Ship tasks through the pipeline
    Ship(ship::ShipCommand),
    /// Inspect cross-agent duel history and scoreboards
    Duel(duel::DuelCommand),
    /// Run an arbitrary job by ID
    Job(job::JobRunArgs),
}

impl Execute for RunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            RunSubcommand::Ship(command) => command.execute(runtime),
            RunSubcommand::Duel(command) => command.execute(runtime),
            RunSubcommand::Job(command) => command.execute(runtime),
        }
    }
}

#[derive(Args, Default)]
pub struct PositionalJobArgs {
    /// Run the named job directly (equivalent to `orbit run job <JOB_ID>`)
    pub job_id: Option<String>,

    /// Input key=value pairs passed to all job steps (repeatable)
    #[arg(long)]
    pub input: Vec<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Stream agent stderr to the terminal and tee stdout live for debugging
    #[arg(long)]
    pub debug: bool,
}

fn execute_positional_job(
    args: PositionalJobArgs,
    runtime: &OrbitRuntime,
) -> Result<(), OrbitError> {
    let Some(job_id) = args.job_id else {
        return Err(OrbitError::InvalidInput(
            "`orbit run` expects a workflow subcommand or job ID".to_string(),
        ));
    };

    ensure_positional_job_exists(runtime, &job_id)?;

    job::JobRunArgs {
        job_id,
        input: args.input,
        backend: None,
        json: args.json,
        debug: args.debug,
    }
    .execute(runtime)
}

fn ensure_positional_job_exists(runtime: &OrbitRuntime, job_id: &str) -> Result<(), OrbitError> {
    match runtime.show_job_catalog_entry(job_id) {
        Ok(_) => Ok(()),
        Err(OrbitError::JobNotFound(_)) => Err(OrbitError::InvalidInput(format!(
            "unknown `orbit run` target `{job_id}`\navailable subcommands: ship, duel, job\ntip: use `orbit job list` to discover valid job ids"
        ))),
        Err(error) => Err(error),
    }
}
