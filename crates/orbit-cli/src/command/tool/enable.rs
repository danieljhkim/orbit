use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute};

#[derive(Args)]
pub struct ToolEnableArgs {
    /// Tool name to enable
    pub name: String,
}

impl Execute for ToolEnableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        runtime.enable_tool(&self.name)?;
        println!("Enabled tool '{}'", self.name);
        Ok(CommandOutput::Silent)
    }
}
