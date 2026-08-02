use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::json;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
pub struct LearningPruneArgs {
    /// Report stale learnings without modifying state (default behaviour).
    #[arg(long = "stale-only", default_value_t = true)]
    pub stale_only: bool,
    /// Archive every stale learning (sets status=superseded, superseded_by=null).
    #[arg(long, visible_alias = "delete", conflicts_with = "stale_only")]
    pub confirm: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningPruneArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let (stale, deleted) = runtime.prune_learnings(self.confirm)?;
        if self.json {
            Ok(Payload::document(json!({
                "stale": stale,
                "deleted": deleted,
            }))
            .into())
        } else {
            if stale.is_empty() {
                println!("No stale learnings.");
            } else {
                println!("Stale learnings ({}):", stale.len());
                for id in &stale {
                    println!("  {id}");
                }
            }
            if !deleted.is_empty() {
                println!("Archived {} stale learning(s).", deleted.len());
            }
            Ok(CommandOutput::Silent)
        }
    }
}
