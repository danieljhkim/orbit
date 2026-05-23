use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::{policy_json, print_policy};

#[derive(Args)]
pub struct PolicyShowArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for PolicyShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let def = runtime
            .get_policy_def(&self.name)?
            .ok_or_else(|| OrbitError::InvalidInput(format!("policy not found: {}", self.name)))?;

        if self.json {
            crate::output::json::print_pretty(&policy_json(&def)?)
        } else {
            print_policy(&def)?;
            Ok(())
        }
    }
}
