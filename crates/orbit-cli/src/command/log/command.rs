use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::tail;

#[derive(Args)]
#[command(about = "Inspect the unified Orbit log feed")]
pub struct LogCommand {
    #[command(subcommand)]
    pub command: LogSubcommand,
}

impl Execute for LogCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum LogSubcommand {
    /// Tail the unified Orbit log feed (`~/.orbit/state/logs/orbit.jsonl`)
    Tail(tail::TailArgs),
}

impl Execute for LogSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            LogSubcommand::Tail(args) => args.execute(runtime),
        }
    }
}
