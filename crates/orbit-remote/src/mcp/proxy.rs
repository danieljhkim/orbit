//! Byte-faithful stdio proxy from a local MCP client to a remote Orbit server.
//!
//! The proxy owns no Orbit execution policy. It starts one non-interactive SSH
//! process for the MCP session and lets that process inherit stdin, stdout, and
//! stderr. Workspace selection, checkout resolution, tool discovery, and any
//! later authorization decisions therefore happen only in the remote server.

use std::path::Path;
use std::process::{Command, Stdio};

use orbit_common::types::OrbitError;
use orbit_common::utility::ssh_tunnel::shell_quote;
use orbit_registry::{HostIdentityState, inspect_host_identity};

/// Audit-only identity used when this machine has no persisted Orbit identity.
pub(super) const LOCAL_CALLER_MACHINE_ID_FALLBACK: &str = "host/local";

/// What `orbit mcp serve --mode remote` was asked to do.
#[derive(Debug, Clone)]
pub struct RemoteProxyArgs {
    /// SSH destination accepted by `ssh`, such as a host, `user@host`, or a
    /// configured alias.
    pub ssh_host: String,
}

/// Relay this process's MCP stdio directly through one non-PTY SSH child.
pub fn serve_mcp_remote_proxy(args: RemoteProxyArgs) -> Result<(), OrbitError> {
    let caller_machine_id = local_caller_machine_id();
    let mut command = ssh_command(&args, &caller_machine_id);
    tracing::info!(
        ssh_host = %args.ssh_host,
        caller_machine_id = %caller_machine_id,
        "starting direct SSH MCP proxy"
    );

    let status = command.status().map_err(|error| {
        OrbitError::Execution(format!(
            "could not start SSH MCP proxy to '{}': {error}",
            args.ssh_host
        ))
    })?;
    if status.success() {
        return Ok(());
    }

    Err(OrbitError::Execution(format!(
        "SSH MCP proxy to '{}' exited with status {status}",
        args.ssh_host
    )))
}

/// Build the exact child process used by the proxy.
///
/// `-T` is load-bearing: allocating a PTY can echo input and transform line or
/// control bytes, corrupting the MCP JSON-RPC stream. Explicit inherited stdio
/// keeps this process out of the protocol entirely.
pub(super) fn ssh_command(args: &RemoteProxyArgs, caller_machine_id: &str) -> Command {
    let mut command = Command::new("ssh");
    command
        .arg("-T")
        .arg("--")
        .arg(&args.ssh_host)
        .arg(remote_serve_command(caller_machine_id))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

/// Remote command whose hidden argument marks an SSH-originated MCP session.
pub(super) fn remote_serve_command(caller_machine_id: &str) -> String {
    format!(
        "orbit mcp serve --remote-caller-machine-id {}",
        shell_quote(caller_machine_id)
    )
}

/// Resolve the caller's persisted machine identity, or the audit-only fallback.
///
/// Identity is metadata, not a credential, so an absent or unreadable local
/// identity must not prevent a client from reaching the authoritative server.
pub(super) fn caller_machine_id_at(global_root: Option<&Path>) -> String {
    let state = global_root.map(inspect_host_identity);
    match state {
        Some(Ok(HostIdentityState::Present(identity))) => identity.machine_id,
        Some(Ok(HostIdentityState::Legacy {
            machine_id: Some(machine_id),
            ..
        })) => machine_id,
        Some(Err(error)) => {
            tracing::warn!(%error, "could not read local Orbit machine identity; using audit fallback");
            LOCAL_CALLER_MACHINE_ID_FALLBACK.to_string()
        }
        Some(Ok(HostIdentityState::Legacy {
            machine_id: None, ..
        }))
        | Some(Ok(HostIdentityState::Absent))
        | None => LOCAL_CALLER_MACHINE_ID_FALLBACK.to_string(),
    }
}

fn local_caller_machine_id() -> String {
    let global_root = orbit_common::utility::path::global_orbit_dir().ok();
    caller_machine_id_at(global_root.as_deref())
}
