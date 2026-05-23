use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::parse::parse_since;

#[derive(Args)]
pub struct AuditPruneArgs {
    /// Prune events older than this duration (e.g. "90d", "1h")
    #[arg(long)]
    pub older_than: String,
}

impl Execute for AuditPruneArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let cutoff = parse_since(&self.older_than)?;
        let pruned = runtime.prune_audit_events(&cutoff)?;
        println!("Pruned {pruned} audit events");
        Ok(())
    }
}
