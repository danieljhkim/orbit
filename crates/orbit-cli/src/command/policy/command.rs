use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::check::PolicyCheckArgs;
use super::list::PolicyListArgs;
use super::show::PolicyShowArgs;

#[derive(Args)]
#[command(about = "Manage filesystem profile policies")]
pub struct PolicyCommand {
    #[command(subcommand)]
    pub command: PolicySubcommand,
}

impl Execute for PolicyCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum PolicySubcommand {
    /// List all policy definitions
    List(PolicyListArgs),
    /// Show a specific policy definition
    Show(PolicyShowArgs),
    /// Dry-run a path against the active policy's fsProfile rules
    Check(PolicyCheckArgs),
}

impl Execute for PolicySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            PolicySubcommand::List(args) => args.execute(runtime),
            PolicySubcommand::Show(args) => args.execute(runtime),
            PolicySubcommand::Check(args) => args.execute(runtime),
        }
    }
}
