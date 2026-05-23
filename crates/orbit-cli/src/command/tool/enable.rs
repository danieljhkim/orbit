use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct ToolEnableArgs {
    /// Tool name to enable
    pub name: String,
}

impl Execute for ToolEnableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.enable_tool(&self.name)?;
        println!("Enabled tool '{}'", self.name);
        Ok(())
    }
}
