use std::net::SocketAddr;
use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use orbit_common::types::McpCapability;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_remote::{DEFAULT_REMOTE_MCP_PORT, McpServerRole, RemoteProxyArgs};

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
    /// Serve the Orbit tool registry over Model Context Protocol
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

/// Which side of the wire this invocation is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ServeMode {
    /// Client side: present stdio MCP locally and relay it to a remote
    /// loopback listener over an SSH tunnel.
    Remote,
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {
    /// Serve the checkoutless owner endpoint for the workspaces this machine
    /// owns, instead of the client-facing local broker.
    ///
    /// ORB-10727 [ADR-0355]: this replaces `--hub`. It selects which server
    /// this process presents; it no longer asserts a machine-level coordination
    /// role, and `host.toml` mode is not consulted. Orbit constructs this
    /// invocation itself for the far side of an owner route — it is not
    /// something a client config should name.
    #[arg(long)]
    pub owner: bool,
    /// Exact non-hierarchical capability for this broker or owner session. With
    /// `--mode remote` this is the capability the remote listener is started
    /// with, and only when this invocation is the one that starts it.
    #[arg(long, value_name = "CAPABILITY")]
    pub capabilities: Option<McpCapability>,
    /// Serve over TCP at this loopback address instead of stdio. A
    /// non-loopback address is refused before the socket is opened, since the
    /// listener carries no authentication of its own; reach it through an
    /// authenticated SSH tunnel (ADR-0350). Stdio remains the default when
    /// this is omitted.
    #[arg(long, value_name = "ADDR", conflicts_with = "mode")]
    pub listen: Option<SocketAddr>,
    /// Run as a client-side proxy to a remote Orbit instead of serving this
    /// machine. Requires an SSH destination, and refuses to start where a
    /// local checkout exists (ADR-0350).
    #[arg(
        long,
        value_name = "MODE",
        requires = "ssh_host",
        conflicts_with = "owner"
    )]
    pub mode: Option<ServeMode>,
    /// SSH destination for `--mode remote` — anything `ssh` accepts (`host`,
    /// `user@host`, or a `~/.ssh/config` alias).
    #[arg(value_name = "SSH_HOST", requires = "mode")]
    pub ssh_host: Option<String>,
    /// Loopback port the remote MCP listener serves on, for `--mode remote`.
    #[arg(long, value_name = "PORT", default_value_t = DEFAULT_REMOTE_MCP_PORT)]
    pub remote_port: u16,
    /// Local port for the `--mode remote` tunnel. Defaults to an ephemeral
    /// port, which is normally right: nothing else needs to find it.
    #[arg(long, value_name = "PORT")]
    pub local_port: Option<u16>,
}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> CommandOut {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp serve does not accept a workspace root override; select a workspace per initialize or tool call"
                    .to_string(),
            ));
        }
        let role = if self.owner {
            McpServerRole::Owner
        } else {
            McpServerRole::Broker
        };
        match (self.mode, self.listen) {
            (Some(ServeMode::Remote), _) => {
                // Clap enforces the pairing; this keeps the invariant local
                // rather than trusting a derive attribute at a distance.
                let ssh_host = self.ssh_host.ok_or_else(|| {
                    OrbitError::InvalidInput(
                        "`orbit mcp serve --mode remote` needs an SSH destination, e.g. \
                         `orbit mcp serve --mode remote my-box`"
                            .to_string(),
                    )
                })?;
                orbit_remote::serve_mcp_remote_proxy(RemoteProxyArgs {
                    ssh_host,
                    remote_port: self.remote_port,
                    local_port: self.local_port,
                    capability: self.capabilities,
                })?
            }
            (None, Some(addr)) => orbit_remote::serve_mcp_tcp(addr, role, self.capabilities)?,
            (None, None) => orbit_remote::serve_mcp_stdio(role, self.capabilities)?,
        }
        Ok(CommandOutput::Silent)
    }
}
