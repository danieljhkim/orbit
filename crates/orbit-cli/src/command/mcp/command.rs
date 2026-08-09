use std::path::Path;

use clap::{Args, Subcommand};
use orbit_common::types::McpCapability;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::{CommandOut, CommandOutput, Execute};

use super::setup::{InitArgs, RemoveArgs};

#[derive(Args)]
#[command(
    about = "Register MCP client integrations and run the MCP server",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

impl Execute for McpCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Initialize MCP client integration for the current workspace
    Init(InitArgs),
    /// Remove MCP client integration for the current workspace
    Remove(RemoveArgs),
    /// Serve the Orbit tool registry over Model Context Protocol on stdio
    Serve(ServeArgs),
}

impl Execute for McpSubcommand {
    fn execute(self, _runtime: &OrbitRuntime) -> CommandOut {
        match self {
            // All MCP subcommands are dispatched runtime-free via main.rs's
            // pattern match before runtime initialization. They reach this
            // path only if invoked indirectly (currently never), so use the
            // same runtime-less call chain for safety.
            Self::Init(args) => args.execute_without_runtime(None),
            Self::Remove(args) => args.execute_without_runtime(None),
            Self::Serve(args) => args.execute_without_runtime(None),
        }
    }
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol on stdio")]
pub struct ServeArgs {
    /// Serve only checkoutless coordination tools as the fixed local hub.
    #[arg(long)]
    pub hub: bool,
    /// Exact non-hierarchical capability for this broker or hub session.
    #[arg(long, value_name = "CAPABILITY")]
    pub capabilities: Option<McpCapability>,
}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> CommandOut {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp serve does not accept a workspace root override; select a workspace per initialize or tool call"
                    .to_string(),
            ));
        }
        {
            orbit_remote::serve_mcp_stdio(self.hub, self.capabilities)?;
            Ok(CommandOutput::Silent)
        }
    }
}
