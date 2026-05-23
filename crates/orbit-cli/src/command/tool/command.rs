use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::add::ToolAddArgs;
use super::disable::ToolDisableArgs;
use super::doctor::execute_doctor;
use super::enable::ToolEnableArgs;
use super::list::ToolListArgs;
use super::remove::ToolRemoveArgs;
use super::run::ToolRunArgs;
use super::scaffold::ToolScaffoldArgs;
use super::show::ToolShowArgs;

const TOOL_COMMAND_AFTER_HELP: &str = "\
Examples:
  orbit tool scaffold ./plugins/hello_orbit.py --name demo.hello
  orbit tool add ./plugins/hello_orbit.py
  orbit tool show demo.hello
";

#[derive(Args)]
#[command(
    about = "Manage and run Orbit tools, including external MCP plugins",
    after_help = TOOL_COMMAND_AFTER_HELP
)]
pub struct ToolCommand {
    #[command(subcommand)]
    pub command: ToolSubcommand,
}

impl Execute for ToolCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ToolSubcommand {
    /// List all registered tools
    List(ToolListArgs),
    /// Show detailed information about a tool
    Show(ToolShowArgs),
    /// Execute a tool
    Run(ToolRunArgs),
    /// Register an external tool or MCP plugin
    Add(ToolAddArgs),
    /// Generate a starter external tool plugin
    Scaffold(ToolScaffoldArgs),
    /// Remove an external tool
    Remove(ToolRemoveArgs),
    /// Enable a disabled tool
    Enable(ToolEnableArgs),
    /// Disable a tool
    Disable(ToolDisableArgs),
    /// Validate tool health
    Doctor,
}

impl Execute for ToolSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ToolSubcommand::List(args) => args.execute(runtime),
            ToolSubcommand::Show(args) => args.execute(runtime),
            ToolSubcommand::Run(args) => args.execute(runtime),
            ToolSubcommand::Add(args) => args.execute(runtime),
            ToolSubcommand::Scaffold(args) => args.execute(runtime),
            ToolSubcommand::Remove(args) => args.execute(runtime),
            ToolSubcommand::Enable(args) => args.execute(runtime),
            ToolSubcommand::Disable(args) => args.execute(runtime),
            ToolSubcommand::Doctor => execute_doctor(runtime),
        }
    }
}
