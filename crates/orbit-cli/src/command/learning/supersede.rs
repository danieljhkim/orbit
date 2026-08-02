use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::json;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningSupersedeArgs {
    /// Learning ID being superseded
    pub id: String,
    /// Replacement learning ID
    #[arg(long = "with")]
    pub with: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningSupersedeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        runtime.author_learning_supersede(&self.id, &self.with)?;
        let old = runtime.get_learning(&self.id)?;
        let new = runtime.get_learning(&self.with)?;

        if self.json {
            Ok(Payload::document(json!({
                "old": learning_to_json(&old),
                "new": learning_to_json(&new),
            }))
            .into())
        } else {
            println!("{} superseded by {}", old.id, new.id);
            Ok(CommandOutput::Silent)
        }
    }
}
