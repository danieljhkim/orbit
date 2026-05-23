use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::show::ConfigShowArgs;

#[derive(Args)]
#[command(about = "Show or update Orbit configuration")]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

impl Execute for ConfigCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ConfigSubcommand {
    /// Display current configuration values
    Show(ConfigShowArgs),
}

impl Execute for ConfigSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ConfigSubcommand::Show(args) => args.execute(runtime),
        }
    }
}
