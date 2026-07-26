use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::add::AutoTaskAddArgs;
use super::generate::AutoTaskGenerateArgs;
use super::list::AutoTaskListArgs;
use super::show::AutoTaskShowArgs;
use super::toggle::AutoTaskToggleArgs;
use super::update::AutoTaskUpdateArgs;

#[derive(Args)]
#[command(about = "Define and manage recurring auto-task templates")]
pub struct AutoTaskCommand {
    #[command(subcommand)]
    pub command: AutoTaskSubcommand,
}

impl Execute for AutoTaskCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum AutoTaskSubcommand {
    /// Create a new auto-task definition
    Add(AutoTaskAddArgs),
    /// List every auto-task definition in this workspace
    List(AutoTaskListArgs),
    /// Show a single definition by name
    Show(AutoTaskShowArgs),
    /// Update an existing definition (present fields only)
    Update(AutoTaskUpdateArgs),
    /// Enable or disable a definition (the kill-switch; not a delete)
    Toggle(AutoTaskToggleArgs),
    /// Mint a task from a definition now (ignores schedule, dedupe, and
    /// `enabled`; leaves the scheduler cursor untouched)
    Generate(AutoTaskGenerateArgs),
}

impl Execute for AutoTaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            AutoTaskSubcommand::Add(args) => args.execute(runtime),
            AutoTaskSubcommand::List(args) => args.execute(runtime),
            AutoTaskSubcommand::Show(args) => args.execute(runtime),
            AutoTaskSubcommand::Update(args) => args.execute(runtime),
            AutoTaskSubcommand::Toggle(args) => args.execute(runtime),
            AutoTaskSubcommand::Generate(args) => args.execute(runtime),
        }
    }
}
