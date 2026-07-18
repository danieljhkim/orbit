use std::path::Path;
use std::sync::Arc;

use clap::{Args, Subcommand};
use orbit_common::types::ToolSessionContext;
use orbit_core::routines::{HostIdentityState, inspect_host_identity};
use orbit_core::runtime::resolve_global_root;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::McpHost;

use crate::command::Execute;

use super::host::BrokerMcpHost;
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
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Initialize MCP client integration for the current workspace
    Init(InitArgs),
    /// Remove MCP client integration for the current workspace
    Remove(RemoveArgs),
    /// Serve the Orbit tool registry over Model Context Protocol
    Serve(ServeArgs),
}

impl Execute for McpSubcommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
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
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp serve does not accept a workspace root override; select a workspace per initialize or tool call"
                    .to_string(),
            ));
        }
        let global_root = resolve_global_root()?;
        let host: Arc<dyn McpHost> = Arc::new(BrokerMcpHost::new(global_root.clone()));

        let (machine_id, host_id) = match inspect_host_identity(&global_root)? {
            HostIdentityState::Present(identity) => {
                (Some(identity.machine_id), Some(identity.host_id))
            }
            HostIdentityState::Legacy { .. } | HostIdentityState::Absent => (None, None),
        };
        let trusted_context = ToolSessionContext::trusted_local(None, machine_id, host_id);

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

        tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context(host, trusted_context))
    }
}
