use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::json;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

/// `orbit task reindex` — rebuild the registry index from on-disk bundles.
#[derive(Args)]
pub struct TaskReindexArgs {
    /// Task-registry workspace id to reindex (default: current workspace).
    #[arg(long)]
    pub workspace: Option<String>,
    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskReindexArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let outcome = runtime.reindex_tasks(self.workspace.as_deref())?;

        if self.json {
            Ok(Payload::document(json!({
                "workspace_id": outcome.workspace_id,
                "indexed": outcome.indexed,
                "removed_stale": outcome.removed_stale,
                "projection_degraded": outcome.projection.degraded_reason,
            }))
            .into())
        } else {
            println!(
                "reindexed workspace '{}': {} bundle(s), {} stale binding(s) dropped",
                outcome.workspace_id, outcome.indexed, outcome.removed_stale
            );
            if let Some(reason) = &outcome.projection.degraded_reason {
                println!("  warning: projection degraded: {reason}");
            }
            Ok(CommandOutput::Silent)
        }
    }
}
