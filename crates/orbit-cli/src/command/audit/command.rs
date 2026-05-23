use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::export::AuditExportArgs;
use super::list::AuditListArgs;
use super::prune::AuditPruneArgs;
use super::show::AuditShowArgs;
use super::stats::AuditStatsArgs;

#[derive(Args)]
#[command(about = "Query the audit event log")]
pub struct AuditCommand {
    #[command(subcommand)]
    pub command: AuditSubcommand,
}

impl Execute for AuditCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum AuditSubcommand {
    /// List audit events
    List(AuditListArgs),
    /// Show a single audit event
    Show(AuditShowArgs),
    /// Prune old audit events
    Prune(AuditPruneArgs),
    /// Export audit events to file
    Export(AuditExportArgs),
    /// Show audit event statistics
    Stats(AuditStatsArgs),
}

impl Execute for AuditSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            AuditSubcommand::List(args) => args.execute(runtime),
            AuditSubcommand::Show(args) => args.execute(runtime),
            AuditSubcommand::Prune(args) => args.execute(runtime),
            AuditSubcommand::Export(args) => args.execute(runtime),
            AuditSubcommand::Stats(args) => args.execute(runtime),
        }
    }
}
