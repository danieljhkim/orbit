use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct WatchCommand {
    #[command(subcommand)]
    pub command: WatchSubcommand,
}

impl Execute for WatchCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum WatchSubcommand {
    Run,
}

impl Execute for WatchSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            WatchSubcommand::Run => {
                runtime.execute_watch_run_command()?;
                Ok(())
            }
        }
    }
}
