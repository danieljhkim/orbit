use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

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
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let learning = runtime.author_learning_archive(&self.id)?;
        if self.json {
            crate::output::json::print_pretty(&learning_to_json(&learning))
        } else {
            println!("{} archived", learning.id);
            Ok(())
        }
    }
}
