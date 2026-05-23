use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct ToolRemoveArgs {
    /// Tool name to remove
    pub name: String,
}

impl Execute for ToolRemoveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.remove_tool(&self.name)?;
        println!("Removed tool '{}'", self.name);
        Ok(())
    }
}
