use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::task::output::task_to_json_for_runtime;

#[derive(Args)]
pub struct AutoTaskMintArgs {
    /// Definition name
    pub name: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AutoTaskMintArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.auto_task_mint(&self.name)?;

        if self.json {
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("{} {}", task.id, task.title);
            Ok(())
        }
    }
}
