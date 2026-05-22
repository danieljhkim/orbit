use clap::Args;
use orbit_core::{LearningStatus, OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct LearningSyncArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningSyncArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.sync_learnings()?;
        let active = runtime.list_learnings(Some(LearningStatus::Active))?.len();
        let superseded = runtime
            .list_learnings(Some(LearningStatus::Superseded))?
            .len();
        let rebuilt = active + superseded;
        if self.json {
            crate::output::json::print_pretty(&json!({ "rebuilt_count": rebuilt }))
        } else {
            println!("Synced {rebuilt} learnings ({active} active, {superseded} superseded)");
            Ok(())
        }
    }
}
