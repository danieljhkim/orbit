use std::path::Path;
use std::sync::Arc;

use clap::{Args, Subcommand};
use orbit_common::types::ToolSessionContext;
use orbit_core::routines::{HostIdentityState, inspect_host_identity};
use orbit_core::runtime::resolve_global_root;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::McpHost;

use crate::command::Execute;

use super::host::{EmptyMcpHost, RuntimeMcpHost};
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
        let (host, workspace_id): (Arc<dyn McpHost>, Option<String>) =
            match OrbitRuntime::try_initialize_existing(root_override)? {
                Some(runtime) => {
                    let workspace_id = runtime.workspace_id()?;
                    (Arc::new(RuntimeMcpHost::new(runtime)), Some(workspace_id))
                }
                None => {
                    let cwd = std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "<unknown>".to_string());
                    eprintln!(
                        "orbit mcp serve: no initialized Orbit workspace discovered from {cwd}; serving empty tool surface"
                    );
                    (Arc::new(EmptyMcpHost), None)
                }
            };

        let (machine_id, host_id) = match inspect_host_identity(&resolve_global_root()?)? {
            HostIdentityState::Present(identity) => {
                (Some(identity.machine_id), Some(identity.host_id))
            }
            HostIdentityState::Legacy { .. } | HostIdentityState::Absent => (None, None),
        };
        let trusted_context = ToolSessionContext::trusted_local(workspace_id, machine_id, host_id);

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

        tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context(host, trusted_context))
    }
}
