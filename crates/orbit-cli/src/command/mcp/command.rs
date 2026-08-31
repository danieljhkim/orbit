use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Args, Subcommand, ValueEnum};
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::{McpSessionAuthority, RemoteProxyArgs, SshAcceptance};

use crate::command::{CommandOut, CommandOutput, Execute};

use super::callers::CallersArgs;
use super::listen::ListenArgs;
use super::setup::{InitArgs, RemoveArgs};

/// True only when the process environment was made non-observable before CLI
/// parsing. A public flag cannot set this bit.
static SSH_ACCEPTANCE_ENV_SEALED: AtomicBool = AtomicBool::new(false);

/// Seal an sshd-provided acceptance bearer before any ordinary CLI startup.
///
/// Linux exposes another same-UID process's initial environment through
/// `/proc/<pid>/environ` while the process is dumpable. `PR_SET_DUMPABLE=0`
/// moves this process behind the kernel's ptrace-access check. The call happens
/// at the first line of `main`, before logging, signal setup, or argument
/// parsing. Processes without the bearer have their previous dumpable state
/// restored immediately, so ordinary Orbit commands retain normal debugging
/// and core-dump behavior.
pub(crate) fn seal_ssh_acceptance_environment() {
    #[cfg(target_os = "linux")]
    {
        // Safety: these prctl operations read or set one process-local integer
        // attribute and do not dereference any pointers.
        let previous = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };
        let sealed = previous >= 0
            // Safety: see above. Zero is the kernel-defined non-dumpable state.
            && unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } == 0;
        if std::env::var_os(orbit_mcp::SSH_ACCEPTANCE_ENV).is_some() {
            SSH_ACCEPTANCE_ENV_SEALED.store(sealed, Ordering::Release);
        } else if sealed && previous != 0 {
            // Safety: restore the process attribute captured above.
            let _ = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, previous) };
        }
    }
}

/// Read the Tier 2 bearer only after the kernel has hidden this process's
/// environment from ordinary processes under the destination login UID.
fn sealed_ssh_acceptance_token() -> Result<String, OrbitError> {
    if !SSH_ACCEPTANCE_ENV_SEALED.load(Ordering::Acquire) {
        return Err(OrbitError::UnauthorizedCaller(
            "SSH MCP acceptance environment is not protected on this host; Tier 2 requires the \
             generated isolated-account SSH deployment on Linux"
                .to_string(),
        ));
    }
    std::env::var(orbit_mcp::SSH_ACCEPTANCE_ENV).map_err(|_| {
        OrbitError::UnauthorizedCaller(
            "SSH MCP acceptance was not supplied by sshd; regenerate and install the \
             authorized_keys line with `orbit mcp callers authorize`"
                .to_string(),
        )
    })
}

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
    ///
    /// By default, registers agent-only authority — the same as bare `orbit
    /// mcp serve`. `--federated` instead adds a separate client entry for the
    /// session-unbound mux and preserves the default v1 entry. For the
    /// operator-authorized bootstrap integration (workflow dispatch,
    /// `orbit.command.exec`), use `orbit workspace init --mcp` instead.
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
    /// Inspect and seed which callers this machine serves, and as what
    ///
    /// On an SSH destination the caller writes the remote argv, so the
    /// authority a remote session asks for is a request. `~/.orbit/mcp-callers.toml`
    /// is this machine's answer, and a remote session holds the intersection
    /// of the two. Local sessions are unaffected.
    Callers(CallersArgs),
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
            Self::Callers(args) => args.execute_without_runtime(None),
        }
    }
}

/// Which side of the wire this invocation is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ServeMode {
    /// Present stdio MCP locally and relay it directly to a remote Orbit over
    /// one non-PTY SSH process.
    Remote,
    /// Present one stdio MCP surface over this machine's workspaces plus every
    /// SSH destination configured in `~/.orbit/mcp-destinations.toml`.
    ///
    /// Local workspaces are included automatically and need no destination
    /// row. This mode binds to no single workspace. It lists each destination's
    /// workspaces as live descriptors, probing remotes on every call, and
    /// includes remotes that are unreachable right now rather than hiding
    /// them. Workspace-scoped tools take the host-qualified `selector` copied
    /// from that list; a registered name, a bare `ws_*`, or `--workspace` is
    /// not a federated selector.
    Federated,
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {
    /// Run as a client to other Orbit servers instead of serving this machine.
    ///
    /// `remote` proxies one chosen SSH destination and requires it as an
    /// argument. `federated` includes this machine automatically and muxes
    /// additional destinations configured in `~/.orbit/mcp-destinations.toml`;
    /// it takes no argument.
    #[arg(long, value_name = "MODE")]
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
    ///
    /// On a session that arrived over SSH this is a *request*, not a grant.
    /// The machine serving it caps the session at what its
    /// `~/.orbit/mcp-callers.toml` grants the calling machine, so a caller
    /// cannot serve itself operator authority on someone else's host. See
    /// `orbit mcp callers check`.
    #[arg(long, conflicts_with = "mode")]
    pub operator: bool,
    /// Treat this session as SSH-originated after validating the destination
    /// capability carried in sshd-provided protected process state.
    ///
    /// Meaningful only inside the forced command printed by `orbit mcp callers
    /// authorize`. The bearer is deliberately not an argument value: the
    /// generated key entry supplies it through an environment option, and
    /// Orbit seals that environment before parsing argv.
    #[arg(long, hide = true, conflicts_with = "mode")]
    pub accept_ssh: bool,
    /// The calling machine's identity, as this machine wrote it beside the
    /// authenticating key.
    ///
    /// Honored only together with `--accept-ssh`, because only there is it
    /// this machine's own statement rather than a string a caller could type.
    /// It selects the `~/.orbit/mcp-callers.toml` row that caps the session,
    /// and unlike a forwarded audit label it is backed by the key sshd checked.
    #[arg(
        long,
        value_name = "MACHINE_ID",
        requires = "accept_ssh",
        conflicts_with = "mode"
    )]
    pub caller: Option<String>,
    /// Bind this server's sessions to a registered workspace: a workspace
    /// name, a logical workspace ID (`ws_*`), or an absolute registered
    /// checkout path.
    ///
    /// Most MCP clients cannot announce `_meta.orbit.workspace` on their
    /// initialize, so without this a workspace-scoped tool must repeat the
    /// selector on every call. Local `orbit mcp init` and `orbit workspace
    /// init --mcp` write this into the integrations they generate; a managed
    /// child that launches `orbit mcp serve` without the flag still inherits
    /// `ORBIT_WORKSPACE` from the trusted execution envelope. The federated
    /// init path is session-unbound. A client that does announce a workspace,
    /// and any explicit per-call `workspace`, still take precedence. The
    /// selector is resolved against the accepting server's registry per call,
    /// never from the server process cwd.
    #[arg(long, value_name = "SELECTOR", conflicts_with = "mode")]
    pub workspace: Option<String>,
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
                // Each mode owns its own argument rule, so the pairing lives
                // here rather than in a derive attribute that cannot express
                // "required by one mode and refused by the other".
                let ssh_host = self.ssh_host.ok_or_else(|| {
                    OrbitError::InvalidInput(
                        "`orbit mcp serve --mode remote` needs an SSH destination, e.g. \
                         `orbit mcp serve --mode remote my-box`"
                            .to_string(),
                    )
                })?;
                orbit_mcp::serve_mcp_remote_proxy(RemoteProxyArgs { ssh_host })?
            }
            Some(ServeMode::Federated) => {
                if let Some(ssh_host) = self.ssh_host {
                    return Err(OrbitError::InvalidInput(format!(
                        "`orbit mcp serve --mode federated` takes no SSH destination, but got \
                         '{ssh_host}'; destinations are configured in \
                         `~/.orbit/mcp-destinations.toml`"
                    )));
                }
                super::server::serve_mcp_federated_stdio()?
            }
            None => super::server::serve_mcp_stdio(
                self.remote_caller_machine_id,
                if self.operator {
                    McpSessionAuthority::Operator
                } else {
                    McpSessionAuthority::Agent
                },
                self.workspace
                    .or_else(orbit_core::runtime::managed_workspace_selector_from_env),
                // `--caller` is unreachable without `--accept-ssh`, so the
                // "honored only under a forced command" rule is carried by the
                // type the server receives rather than re-checked downstream.
                if self.accept_ssh {
                    SshAcceptance::ForcedCommand {
                        caller: self.caller,
                        acceptance_token: sealed_ssh_acceptance_token()?,
                    }
                } else {
                    SshAcceptance::Environment
                },
            )?,
        }
        Ok(CommandOutput::Silent)
    }
}
