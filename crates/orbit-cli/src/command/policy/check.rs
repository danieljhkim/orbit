use clap::Args;
use orbit_common::types::DEFAULT_POLICY_NAME;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

use super::support::status_word;

#[derive(Args)]
pub struct PolicyCheckArgs {
    pub profile_name: String,
    pub path: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for PolicyCheckArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let def = runtime
            .get_policy_def(DEFAULT_POLICY_NAME)?
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!("policy not found: {}", DEFAULT_POLICY_NAME))
            })?;

        let read = def.check_path(
            &self.profile_name,
            orbit_common::types::FsOperation::Read,
            &self.path,
        )?;
        let modify = def.check_path(
            &self.profile_name,
            orbit_common::types::FsOperation::Modify,
            &self.path,
        )?;

        if self.json {
            return crate::output::json::print_pretty(&json!({
                "policy": DEFAULT_POLICY_NAME,
                "profile": self.profile_name,
                "path": self.path,
                "read": {
                    "allowed": read.allowed,
                    "matched_rule": read.matched_rule,
                },
                "modify": {
                    "allowed": modify.allowed,
                    "matched_rule": modify.matched_rule,
                },
            }));
        }

        println!("Policy:  {}", DEFAULT_POLICY_NAME);
        println!("Profile: {}", self.profile_name);
        println!("Path:    {}", self.path);
        println!(
            "read:    {} ({})",
            status_word(read.allowed),
            read.matched_rule
        );
        println!(
            "modify:  {} ({})",
            status_word(modify.allowed),
            modify.matched_rule
        );
        Ok(())
    }
}
