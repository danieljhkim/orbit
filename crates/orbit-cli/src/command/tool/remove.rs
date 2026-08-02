use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
pub struct ToolRemoveArgs {
    /// Tool name to remove
    pub name: String,
}

impl Execute for ToolRemoveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        runtime.remove_tool(&self.name)?;
        println!("Removed tool '{}'", self.name);
        Ok(CommandOutput::Silent)
    }
}
