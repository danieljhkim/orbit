use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::run::{JobReplayArgs, JobRunArgs, JobRunPipelineWorkerArgs};

use super::list::JobListArgs;
use super::show::JobShowArgs;

#[derive(Args)]
#[command(about = "Define, list, and manage job workflows")]
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
    /// List all registered jobs
    List(JobListArgs),
    /// Show details of a specific job
    Show(JobShowArgs),
    /// Execute a schemaVersion 2 job by ID or YAML path
    Run(JobRunArgs),
    /// Replay a previous job run from step 0 using the current job definition
    Replay(JobReplayArgs),
    /// Internal worker entrypoint for persisted pipeline runs
    #[command(name = "run-pipeline-worker", hide = true)]
    RunPipelineWorker(JobRunPipelineWorkerArgs),
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::List(args) => args.execute(runtime),
            JobSubcommand::Show(args) => args.execute(runtime),
            JobSubcommand::Run(args) => args.execute(runtime),
            JobSubcommand::Replay(args) => args.execute(runtime),
            JobSubcommand::RunPipelineWorker(args) => args.execute(runtime),
        }
    }
}
