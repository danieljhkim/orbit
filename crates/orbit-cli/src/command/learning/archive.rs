use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningArchiveArgs {
    /// Learning ID to retire without a replacement
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningArchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let learning = runtime.author_learning_archive(&self.id)?;
        if self.json {
            Ok(Payload::document(learning_to_json(&learning)).into())
        } else {
            println!("{} archived", learning.id);
            Ok(CommandOutput::Silent)
        }
    }
}
