use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::{McpSessionAuthority, RemoteProxyArgs};

use crate::command::{CommandOut, CommandOutput, Execute};

use super::listen::ListenArgs;
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
    /// Serve the Orbit tool registry over Model Context Protocol on a TCP socket
    ///
    /// This is the transport for deployments that need a socket — a server-side
    /// Orbit reached through an SSH tunnel, for example. `orbit mcp serve`
    /// remains the stdio server that MCP clients launch directly.
    ///
    /// Each accepted connection is an independent MCP session against the same
    /// server-local tool surface, resolved and audited exactly as a stdio session
    /// is. The socket authenticates no client, so it binds loopback unless a
    /// wider bind is asked for explicitly.
    Listen(ListenArgs),
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
            Self::Listen(args) => args.execute_without_runtime(None),
        }
    }
}

/// Which side of the wire this invocation is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ServeMode {
    /// Present stdio MCP locally and relay it directly to a remote Orbit over
    /// one non-PTY SSH process.
    Remote,
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {
    /// Run as a client-side proxy to a remote Orbit instead of serving this
    /// machine. Requires an SSH destination.
    #[arg(long, value_name = "MODE", requires = "ssh_host")]
    pub mode: Option<ServeMode>,
    /// SSH destination for `--mode remote`, such as a host, `user@host`, or a
    /// configured alias.
    #[arg(value_name = "SSH_HOST", requires = "mode")]
    pub ssh_host: Option<String>,
    /// Audit identity supplied only by Orbit's direct SSH proxy command.
    /// Presence also marks the server session's transport as SSH MCP.
    #[arg(long, value_name = "MACHINE_ID", hide = true, conflicts_with = "mode")]
    pub remote_caller_machine_id: Option<String>,
    /// Serve sessions with operator authority, so they may perform governed
    /// operations such as dispatching a workflow or deleting a task.
    ///
    /// Omit this for any server an agent launches: without it a session holds
    /// the agent capability only, and governed operations are refused. The flag
    /// is the deliberate act — `ORBIT_OPERATOR` in this process's environment is
    /// ignored on the MCP surface, so an operator shell cannot grant operator
    /// authority to an agent's server by accident.
    #[arg(long, conflicts_with = "mode")]
    pub operator: bool,
}

impl ServeArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> CommandOut {
        if root_override.is_some() {
            return Err(OrbitError::InvalidInput(
                "orbit mcp serve does not accept a workspace root override; select a workspace per initialize or tool call"
                    .to_string(),
            ));
        }
        match self.mode {
            Some(ServeMode::Remote) => {
                // Clap enforces the pairing; this keeps the invariant local
                // rather than trusting a derive attribute at a distance.
                let ssh_host = self.ssh_host.ok_or_else(|| {
                    OrbitError::InvalidInput(
                        "`orbit mcp serve --mode remote` needs an SSH destination, e.g. \
                         `orbit mcp serve --mode remote my-box`"
                            .to_string(),
                    )
                })?;
                orbit_mcp::serve_mcp_remote_proxy(RemoteProxyArgs { ssh_host })?
            }
            None => super::server::serve_mcp_stdio(
                self.remote_caller_machine_id,
                if self.operator {
                    McpSessionAuthority::Operator
                } else {
                    McpSessionAuthority::Agent
                },
            )?,
        }
        Ok(CommandOutput::Silent)
    }
}
