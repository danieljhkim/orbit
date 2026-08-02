use clap::{Args, Subcommand};
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, Execute};

use super::list::ExecutorListArgs;
use super::show::ExecutorShowArgs;

#[derive(Args)]
#[command(about = "Manage executors")]
pub struct ExecutorCommand {
    #[command(subcommand)]
    pub command: ExecutorSubcommand,
}

impl Execute for ExecutorCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ExecutorSubcommand {
    /// List all executor definitions
    List(ExecutorListArgs),
    /// Show a specific executor definition
    Show(ExecutorShowArgs),
}

impl Execute for ExecutorSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        match self {
            ExecutorSubcommand::List(args) => args.execute(runtime),
            ExecutorSubcommand::Show(args) => args.execute(runtime),
        }
    }
}
