use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
pub struct ToolDisableArgs {
    /// Tool name to disable
    pub name: String,
}

impl Execute for ToolDisableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        runtime.disable_tool(&self.name)?;
        println!("Disabled tool '{}'", self.name);
        Ok(CommandOutput::Silent)
    }
}
