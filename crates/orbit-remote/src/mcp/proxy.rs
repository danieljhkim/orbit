//! Tunnelled stdio proxy mode for a checkoutless client [ORB-10710].
//!
//! `orbit mcp serve --mode remote <ssh-alias>` presents an ordinary stdio MCP
//! server to whatever launched it and relays every byte to a loopback-bound
//! `orbit mcp serve --listen` on the far side of an SSH tunnel ([ADR-0350],
//! [mcp-bridge/2_design.md §5.3]).
//!
//! # What this is, and is not
//!
//! Orbit's other cross-machine MCP path is the spoke→hub link, which exists
//! because a spoke *has* a checkout: its graph, docs, and search must resolve
//! against the branch its agent is working on, and placement routing guarantees
//! that. A **checkoutless client** — an off-box orchestrator that owns no
//! workspace, whose clone (if it keeps one) is a read mirror, and whose every
//! workspace lives on the remote machine — does not fit that shape. Placement
//! routing protects nothing for it and only makes the canonical surface
//! unreachable.
//!
//! So this is deliberately *not* a hub link and must not acquire hub-link
//! responsibilities: no placement routing, no workspace-ownership resolution,
//! no spoke registration, no `mcp.toml` entry. It reads no trusted
//! configuration and constructs no broker or hub host. Everything resolves on
//! the remote, through the paths that already implement filtering and audit.
//!
//! It also introduces no [`orbit_common::types::McpTransport`] variant. The
//! remote listener already stamps the session it serves as trusted-local, and
//! relaying does not change whose session it is; existing checks compare that
//! discriminant by equality rather than exhaustively, so a new variant would
//! compile cleanly while quietly failing every one of them.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use orbit_common::types::{McpCapability, OrbitError, WorkspaceRegistry};
use orbit_common::utility::ssh_tunnel::{self, TunnelOrigin, TunnelSpec};
use orbit_core::runtime::resolve_global_root;

/// Loopback port the remote `orbit mcp serve --listen` is expected on when the
/// operator does not name one. Adjacent to the dashboard's 7878 so the two
/// remote-access surfaces read as a pair.
pub const DEFAULT_REMOTE_MCP_PORT: u16 = 7879;

/// How long to wait for a freshly started remote listener to accept.
const READINESS_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for an *already-running* remote listener to answer through
/// a bare probe forward before concluding nothing is there.
const ATTACH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-probe TCP connect/read timeout.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// What `orbit mcp serve --mode remote` was asked to do.
#[derive(Debug, Clone)]
pub struct RemoteProxyArgs {
    /// SSH destination — anything `ssh` accepts (`host`, `user@host`, or a
    /// `~/.ssh/config` alias).
    pub ssh_host: String,
    /// Loopback port the remote listener serves on.
    pub remote_port: u16,
    /// Local port for the forward. `None` takes an ephemeral one, which is
    /// what a client-registered proxy normally wants: nothing else needs to
    /// find it.
    pub local_port: Option<u16>,
    /// Capability to start the remote listener with when this invocation is
    /// the one that starts it. Capability is always chosen by whoever starts
    /// the listener, never negotiated by the client afterwards.
    pub capability: Option<McpCapability>,
}

/// Serve a checkoutless client by relaying its stdio to a remote listener.
///
/// Refuses on a machine that owns an Orbit checkout, establishes or
/// attaches to one tunnel, opens one connection through it, and relays until
/// the session ends. The tunnel is torn down on every exit path by
/// [`ssh_tunnel::SshTunnel`]'s `Drop`.
pub fn serve_mcp_remote_proxy(args: RemoteProxyArgs) -> Result<(), OrbitError> {
    ensure_checkoutless_client(&args.ssh_host)?;

    let local_port = match args.local_port {
        Some(port) => ssh_tunnel::require_local_port(port)?,
        None => ssh_tunnel::ephemeral_port()?,
    };
    let spec = tunnel_spec(&args, local_port);
    let (tunnel, origin) =
        ssh_tunnel::establish(&spec, || listener_answers(local_port, PROBE_TIMEOUT))?;
    tracing::info!(
        ssh_host = %args.ssh_host,
        local_port,
        remote_port = args.remote_port,
        attached = matches!(origin, TunnelOrigin::Attached),
        "mcp remote proxy tunnel established"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| OrbitError::Execution(format!("tokio runtime: {error}")))?;
    let result = runtime.block_on(async move {
        // One connection for the whole session. Every call in it rides this
        // stream and the one tunnel underneath; nothing is re-established
        // per call.
        let stream =
            tokio::net::TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, local_port)))
                .await
                .map_err(|error| {
                    OrbitError::Execution(format!(
                        "could not reach the remote MCP listener through the tunnel on \
                 localhost:{local_port}: {error}"
                    ))
                })?;
        orbit_mcp::relay_stdio(stream).await
    });

    drop(tunnel);
    result
}

/// Describe this proxy's tunnel to the shared mechanism.
pub(super) fn tunnel_spec(args: &RemoteProxyArgs, local_port: u16) -> TunnelSpec {
    TunnelSpec {
        ssh_host: args.ssh_host.clone(),
        local_port,
        remote_port: args.remote_port,
        remote_command: remote_listen_command(args.remote_port, args.capability),
        remote_description: "orbit mcp serve --listen".to_string(),
        readiness_target: format!(
            "the remote MCP listener on 127.0.0.1:{} (forwarded to localhost:{local_port})",
            args.remote_port
        ),
        attach_timeout: ATTACH_PROBE_TIMEOUT,
        ready_timeout: READINESS_TIMEOUT,
    }
}

/// The remote shell command line that starts the listener when nothing is
/// already serving it.
///
/// Always a loopback bind: the listener carries no authentication of its own
/// and `serve_mcp_tcp` refuses a routable address anyway, so naming loopback
/// here keeps the two ends stating the same posture.
pub(super) fn remote_listen_command(remote_port: u16, capability: Option<McpCapability>) -> String {
    let mut command = format!("orbit mcp serve --listen 127.0.0.1:{remote_port}");
    if let Some(capability) = capability {
        command.push_str(" --capabilities ");
        command.push_str(&ssh_tunnel::shell_quote(&capability.to_string()));
    }
    command
}

/// Whether something is listening behind the forward on `local_port`.
///
/// `ssh -L` accepts a local connection before it knows whether the remote side
/// will, and closes the forwarded channel when the remote refuses. A bare
/// connect is therefore not evidence; holding the connection open is. An MCP
/// server never speaks first, so a read that blocks past the timeout means a
/// server is there, and an immediate EOF means nothing is.
fn listener_answers(local_port: u16, timeout: Duration) -> bool {
    use std::io::Read;

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err() {
        return false;
    }
    let mut probe = [0u8; 1];
    match stream.read(&mut probe) {
        // The forward closed under us: nothing is listening on the far side.
        Ok(0) => false,
        // Held open past the read deadline — a server is waiting for our
        // initialize, which is exactly what readiness means here.
        Err(error) => matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
        // Nothing should speak first, but something plainly answered.
        Ok(_) => true,
    }
}

/// Refuse to run the proxy on a machine that owns an Orbit checkout.
///
/// This guard is load-bearing rather than a convenience. Without it a spoke
/// could register the mode and receive another machine's branch state as its
/// own — which surfaces as *wrong answers*, not as an error, because every
/// response is well-formed and merely describes the wrong tree.
fn ensure_checkoutless_client(ssh_host: &str) -> Result<(), OrbitError> {
    let registry = match resolve_global_root() {
        Ok(global_root) => {
            let path = crate::workspace_registry::registry_path_for(&global_root);
            crate::workspace_registry::load_registry_from(&path)?
        }
        // No global root means no registry to consult, and the registry is the
        // only thing this guard reads: a machine with nowhere to record a
        // checkout binding owns none.
        Err(_) => WorkspaceRegistry::default(),
    };
    let cwd = std::env::current_dir().ok();

    match local_checkout_evidence(&registry, cwd.as_deref()) {
        None => Ok(()),
        Some(evidence) => Err(owned_checkout_refusal(ssh_host, &evidence)),
    }
}

/// The refusal an owning machine gets: what was found, why it disqualifies the
/// mode, and the surface to register instead.
pub(super) fn owned_checkout_refusal(ssh_host: &str, evidence: &str) -> OrbitError {
    OrbitError::InvalidInput(format!(
        "refusing to start `orbit mcp serve --mode remote {ssh_host}`: this machine owns a local \
         Orbit checkout ({evidence}). The remote mode resolves every workspace on '{ssh_host}', \
         so a machine with its own checkout would receive that machine's branch state as its own \
         — wrong answers rather than an error. Register the ordinary local broker \
         (`orbit mcp serve`) here instead, and use the remote mode only from a client that owns \
         no checkout."
    ))
}

/// Describe the first evidence that this machine *owns* a checkout, or `None`
/// when it owns none.
///
/// Two ordered signals, both registry-sourced: a checkout binding the directory
/// we are standing in — which includes an explicit `path_overrides` entry whose
/// `repo_root` may live elsewhere — and, failing that, any registered checkout
/// still present on disk. A registry entry whose `repo_root` is gone is stale
/// rather than evidence: refusing on it would strand a client over a checkout
/// that no longer exists.
// ADR-0360 — ownership, not filesystem shape, is the discriminator, which is
// why a `.orbit/` directory sitting above `cwd` is not consulted at all. That
// directory is deliberately committed in repos that version their Orbit
// workspace, so it arrives with `git clone` and describes a read mirror just as
// well as a working tree. This machine's workspace registry is the only
// machine-local record of what it coordinates, and it is what the local broker
// itself demands — `McpHost::resolve_exact_checkout` binds a session only to a
// registered checkout — so an unregistered `.orbit/` leaves no local-derived
// state for this guard to protect.
pub(super) fn local_checkout_evidence(
    registry: &WorkspaceRegistry,
    cwd: Option<&Path>,
) -> Option<String> {
    if let Some(cwd) = cwd
        && let Some(checkout) = crate::workspace_registry::find_checkout_by_path(registry, cwd)
    {
        // Name the registered path that actually claims `cwd`, which is the
        // repository root unless an override binds a directory outside it.
        let bound = std::iter::once(&checkout.repo_root)
            .chain(&checkout.path_overrides)
            .filter(|candidate| cwd.starts_with(candidate))
            .max_by_key(|candidate| candidate.as_os_str().len())
            .unwrap_or(&checkout.repo_root);
        return Some(format!(
            "workspace '{}' is registered at {}, which contains the working directory",
            checkout.workspace_id,
            bound.display()
        ));
    }
    crate::workspace_registry::local_workspaces(registry)
        .find(|(_, checkout)| checkout.repo_root.exists())
        .map(|(workspace, checkout)| {
            format!(
                "workspace '{}' is checked out at {}",
                workspace.id,
                checkout.repo_root.display()
            )
        })
}
