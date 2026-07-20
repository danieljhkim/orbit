use clap::Args;
use orbit_core::command::knowledge_policy::KnowledgeOwnerAccess;
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
        if !matches!(
            runtime.knowledge_owner_access(),
            KnowledgeOwnerAccess::Standalone
        ) {
            orbit_remote::validate_local_knowledge_for_sync(
                &runtime.global_root(),
                &runtime.paths().repo_root,
                false,
                true,
            )?;
        }
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
