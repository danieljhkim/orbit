use std::path::PathBuf;

use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
#[command(
    after_help = "Use this when the ADR lives in another checkout and you want to mutate it \
from here: reconcile the bundle in first, then `orbit adr update`. Inside the worktree that already \
owns the ADR, `orbit adr update` works directly and no reconcile is needed."
)]
pub struct AdrReconcileArgs {
    /// Existing canonical ADR ID to reconcile (e.g. `ADR-0184`)
    pub id: String,
    /// Registered Git worktree containing the complete source bundle
    #[arg(long = "source-worktree")]
    pub source_worktree: PathBuf,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrReconcileArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let adr = runtime.reconcile_federated_adr(&self.id, &self.source_worktree)?;
        if self.json {
            let value = serde_json::to_value(&adr)
                .map_err(|error| OrbitError::Execution(error.to_string()))?;
            Ok(Payload::document(value).into())
        } else {
            println!("{}", adr.id);
            Ok(CommandOutput::Silent)
        }
    }
}
