//! Server-local identity facts attached to one MCP stdio session.

use std::path::Path;

use orbit_common::types::{McpTransport, OrbitError, ToolSessionContext};

use orbit_registry::{HostIdentityState, inspect_host_identity, os_hostname};

const LOCAL_MACHINE_FALLBACK: &str = "host/local";
const LOCAL_HOST_FALLBACK: &str = "local";

/// Identity and trusted audit context derived by the accepting machine.
pub struct McpServerIdentity {
    pub process_machine_id: String,
    pub process_host_id: String,
    pub session_context: ToolSessionContext,
}

/// Resolve the accepting machine and build its audit-only session envelope.
///
/// `remote_caller_machine_id` is an opaque label forwarded by the SSH proxy,
/// not an authenticated principal. `SSH_CONNECTION`, when present, contributes
/// only the best-effort caller IP.
pub fn mcp_server_identity(
    global_root: &Path,
    remote_caller_machine_id: Option<String>,
) -> Result<McpServerIdentity, OrbitError> {
    let (process_machine_id, process_host_id) = local_identity(global_root)?;
    let is_remote = remote_caller_machine_id.is_some();
    let caller_machine_id = remote_caller_machine_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| process_machine_id.clone());
    let session_context = ToolSessionContext {
        caller_machine_id: Some(caller_machine_id),
        caller_host_id: (!is_remote).then(|| process_host_id.clone()),
        process_machine_id: Some(process_machine_id.clone()),
        process_host_id: Some(process_host_id.clone()),
        transport: Some(if is_remote {
            McpTransport::SshMcp
        } else {
            McpTransport::Local
        }),
        caller_ip: is_remote.then(observed_ssh_caller_ip).flatten(),
        ..ToolSessionContext::default()
    };
    Ok(McpServerIdentity {
        process_machine_id,
        process_host_id,
        session_context,
    })
}

pub(super) fn local_identity(global_root: &Path) -> Result<(String, String), OrbitError> {
    match inspect_host_identity(global_root)? {
        HostIdentityState::Present(identity) => Ok((identity.machine_id, identity.host_id)),
        HostIdentityState::Legacy { host_id, .. } => {
            Ok((LOCAL_MACHINE_FALLBACK.to_string(), host_id))
        }
        HostIdentityState::Absent => Ok((
            LOCAL_MACHINE_FALLBACK.to_string(),
            os_hostname().unwrap_or_else(|| LOCAL_HOST_FALLBACK.to_string()),
        )),
    }
}

fn observed_ssh_caller_ip() -> Option<String> {
    std::env::var("SSH_CONNECTION")
        .ok()
        .and_then(|connection| ssh_caller_ip(&connection))
}

pub(super) fn ssh_caller_ip(connection: &str) -> Option<String> {
    connection
        .split_whitespace()
        .next()
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty())
}
