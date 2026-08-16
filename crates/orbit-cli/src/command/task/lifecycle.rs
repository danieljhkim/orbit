use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::output::task_to_json_for_runtime;

#[derive(Args)]
pub struct TaskStartArgs {
    /// Task ID
    pub id: String,
    /// Optional lifecycle note (records proposal approval when starting proposed work)
    #[arg(long)]
    pub note: Option<String>,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Crew override for this start
    #[arg(long)]
    pub crew: Option<String>,
    /// Explicit agent model to persist on the task artifact
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskStartArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let (agent, model) = super::mutation_identity(self.model);
        let task = runtime.start_task_with_identity_and_crew(
            &self.id,
            self.note,
            self.comment,
            agent,
            model,
            self.crew,
        )?;
        if self.json {
            Ok(Payload::document(task_to_json_for_runtime(runtime, &task)?).into())
        } else {
            println!("Started task '{}'", task.id);
            Ok(CommandOutput::Silent)
        }
    }
}

#[derive(Args)]
#[command(after_help = "Restore an archived task with `orbit task update <id> --status backlog`.")]
pub struct TaskArchiveArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskArchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        runtime.archive_task(&self.id)?;
        if self.json {
            let task = runtime.get_task(&self.id)?;
            Ok(Payload::document(task_to_json_for_runtime(runtime, &task)?).into())
        } else {
            println!("Archived task '{}'", self.id);
            Ok(CommandOutput::Silent)
        }
    }
}
