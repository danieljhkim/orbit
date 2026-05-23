use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::install::{HookInstallArgs, HookUninstallArgs};
use super::pretooluse::PretooluseArgs;

#[derive(Args)]
#[command(about = "Run Orbit-owned editor hooks")]
pub struct HookCommand {
    #[command(subcommand)]
    pub command: HookSubcommand,
}

impl Execute for HookCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum HookSubcommand {
    /// Install Orbit-owned hook integrations for detected agent directories
    Install(HookInstallArgs),
    /// Inject project-learning reminders for agent PreToolUse hooks
    #[command(name = "pretooluse")]
    Pretooluse(PretooluseArgs),
    /// Remove Orbit-owned hook integrations while preserving user entries
    Uninstall(HookUninstallArgs),
}

impl Execute for HookSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            HookSubcommand::Install(args) => args.execute(runtime),
            HookSubcommand::Pretooluse(args) => args.execute(runtime),
            HookSubcommand::Uninstall(args) => args.execute(runtime),
        }
    }
}
