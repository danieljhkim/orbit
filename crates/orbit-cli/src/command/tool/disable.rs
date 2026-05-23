use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct ToolDisableArgs {
    /// Tool name to disable
    pub name: String,
}

impl Execute for ToolDisableArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.disable_tool(&self.name)?;
        println!("Disabled tool '{}'", self.name);
        Ok(())
    }
}
