use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use clap::{Args, Subcommand};
use orbit_common::types::{McpCapability, ToolSessionContext};
use orbit_core::routines::{HostIdentityState, inspect_host_identity};
use orbit_core::runtime::resolve_global_root;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::McpHost;

use crate::command::Execute;

use super::config::load_trusted_mcp_config;
use super::host::BrokerMcpHost;
use super::hub::HubMcpHost;
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
pub struct ServeArgs {
    /// Serve only checkoutless coordination tools as the fixed local hub.
    #[arg(long)]
    pub hub: bool,
    /// Exact non-hierarchical capability for this hub server session.
    #[arg(long, value_name = "CAPABILITY", requires = "hub")]
    pub capabilities: Option<McpCapability>,
}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp serve does not accept a workspace root override; select a workspace per initialize or tool call"
                    .to_string(),
            ));
        }
        let global_root = resolve_global_root()?;
        // Parse the trusted file, when present, before constructing either
        // server host. Workspace/cwd config never participates in this load.
        let trusted_config = load_trusted_mcp_config(&global_root)?;
        if !self.hub && self.capabilities.is_some() {
            return Err(OrbitError::InvalidInput(
                "--capabilities requires --hub".to_string(),
            ));
        }
        let capability = self.capabilities.unwrap_or(McpCapability::Agent);
        let (host, mut trusted_context): (Arc<dyn McpHost>, ToolSessionContext) = if self.hub {
            let hub = HubMcpHost::new(global_root.clone(), capability)?;
            let identity = hub.identity();
            let context = ToolSessionContext::trusted_local(
                None,
                Some(identity.machine_id.clone()),
                Some(identity.host_id.clone()),
            );
            (Arc::new(hub), context)
        } else {
            let host: Arc<dyn McpHost> = Arc::new(BrokerMcpHost::new(global_root.clone()));
            let (machine_id, host_id) = match inspect_host_identity(&global_root)? {
                HostIdentityState::Present(identity) => {
                    if identity.mode == orbit_core::routines::HostMode::Spoke {
                        let (route, _) = trusted_config.spoke_route(&identity, Some(capability))?;
                        // E1 validates and fixes this route but intentionally
                        // does not open it; the E2 connector consumes it.
                        debug_assert_eq!(route.transport, super::config::HubTransport::Ssh);
                        debug_assert!(!route.host.is_empty());
                    }
                    (Some(identity.machine_id), Some(identity.host_id))
                }
                HostIdentityState::Legacy { .. } | HostIdentityState::Absent => (None, None),
            };
            (
                host,
                ToolSessionContext::trusted_local(None, machine_id, host_id),
            )
        };
        trusted_context.effective_capabilities = BTreeSet::from([capability]);

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

        tokio_runtime.block_on(orbit_mcp::serve_stdio_with_context(host, trusted_context))
    }
}
