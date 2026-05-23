use std::path::PathBuf;

use clap::{Args, Subcommand};
use orbit_common::types::TaskArtifact;
use orbit_core::command::task::TaskUpdateParams;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::output::task_to_json_for_runtime;

#[derive(Args)]
#[command(about = "Manage task artifact files")]
pub struct TaskArtifactCommand {
    #[command(subcommand)]
    pub command: TaskArtifactSubcommand,
}

impl Execute for TaskArtifactCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskArtifactSubcommand {
    /// Store a UTF-8 source file under a task's artifacts directory
    Put(TaskArtifactPutArgs),
}

impl Execute for TaskArtifactSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskArtifactSubcommand::Put(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct TaskArtifactPutArgs {
    /// Task ID
    pub id: String,
    /// UTF-8 source file to store as a task artifact
    pub source_path: PathBuf,
    /// Artifact path relative to the task artifacts directory. Defaults to the source file name.
    #[arg(long = "path")]
    pub artifact_path: Option<String>,
    /// Explicit agent name to persist on the task artifact update
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model to persist on the task artifact update
    #[arg(long)]
    pub model: Option<String>,
    /// Output the updated task as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskArtifactPutArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let TaskArtifactPutArgs {
            id,
            source_path,
            artifact_path,
            agent,
            model,
            json,
        } = self;
        let artifact = TaskArtifact::from_source_file(&source_path, artifact_path.as_deref())?;
        let artifact_path = artifact.path.clone();
        let task = runtime.update_task_with_identity(
            &id,
            TaskUpdateParams {
                upsert_artifacts: vec![artifact],
                ..Default::default()
            },
            agent,
            model,
        )?;

        if json {
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Stored artifact '{artifact_path}' on task '{}'", task.id);
            Ok(())
        }
    }
}
