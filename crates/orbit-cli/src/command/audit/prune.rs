use clap::Args;
use orbit_core::command::gc::{GcRequest, GcScope, SystemGcClock, execute_gc};
use orbit_core::command::gc_audit::AuditGcCollector;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::parse::parse_since;

#[derive(Args)]
pub struct AuditPruneArgs {
    /// Prune events older than this duration (e.g. "90d", "1h")
    #[arg(long)]
    pub older_than: String,

    /// Apply the compatibility GC plan (omitting this is a safe preview)
    #[arg(long)]
    pub apply: bool,
}

impl Execute for AuditPruneArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        // Parse once for compatibility with the historical timestamp syntax;
        // unified GC accepts duration retention, so absolute cutoffs are
        // translated into a conservative whole-second duration.
        let cutoff = parse_since(&self.older_than)?;
        let retention = (chrono::Utc::now() - cutoff).num_seconds().max(0);
        let retention = format!("{retention}s");
        let collector = AuditGcCollector::new(
            runtime.sqlite_store()?,
            runtime.workspace_id()?,
            &runtime.paths().orbit_dir,
        );
        let clock = SystemGcClock;
        let report = execute_gc(
            &collector,
            GcRequest {
                apply: self.apply,
                scope: GcScope::Workspace {
                    workspace_id: Some(runtime.workspace_id()?),
                    root: runtime.paths().orbit_dir.clone(),
                },
                retention_override: Some(&retention),
                global_state_dir: &runtime.paths().global_dir.join("state"),
                clock: &clock,
            },
        )?;
        println!(
            "`orbit audit prune` is deprecated; use `orbit gc audit --retention {}{}`",
            self.older_than,
            if self.apply { " --apply" } else { "" }
        );
        crate::command::gc::print_human_report(&report);
        Ok(())
    }
}
