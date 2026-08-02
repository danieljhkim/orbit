use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::{CommandOut, Execute, Payload};

use super::support::{policy_json, policy_text};

#[derive(Args)]
pub struct PolicyShowArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for PolicyShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let def = runtime
            .get_policy_def(&self.name)?
            .ok_or_else(|| OrbitError::InvalidInput(format!("policy not found: {}", self.name)))?;

        Ok(Payload::detail(policy_json(&def)?, policy_text(&def)?).into())
    }
}
