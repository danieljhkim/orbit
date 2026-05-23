use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::list::ActivityListArgs;

#[derive(Args)]
#[command(about = "List v2 activities")]
pub struct ActivityCommand {
    #[command(subcommand)]
    pub command: ActivitySubcommand,
}

impl Execute for ActivityCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ActivitySubcommand {
    /// List all registered activities
    List(ActivityListArgs),
}

impl Execute for ActivitySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ActivitySubcommand::List(args) => args.execute(runtime),
        }
    }
}
