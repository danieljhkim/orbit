//! Server-local identity facts attached to one MCP stdio session.

use std::collections::BTreeSet;
use std::path::Path;

use orbit_common::OrbitError;
use orbit_types::identity::validate_machine_id;
use orbit_types::tool::{McpCapability, McpTransport, ToolSessionContext};

use orbit_registry::{HostIdentityState, inspect_host_identity, os_hostname};

use super::callers::{RemoteCallerIdentity, SessionCapabilityPolicy};
use super::ssh_auth::{self, SshAcceptance};

const LOCAL_MACHINE_FALLBACK: &str = "host/local";
const LOCAL_HOST_FALLBACK: &str = "local";

/// The authority an MCP server process serves its sessions with.
///
/// This is the only place an MCP session acquires capabilities: the tool
/// chokepoint resolves an MCP call from the session alone, so a server that
/// stamps nothing here can never reach a governed operation. The choice is made
/// once, when the server process starts, and never from the live process
/// environment — an agent that launches its own `orbit mcp serve` gets
/// [`Self::Agent`] no matter what the launching shell exported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpSessionAuthority {
    /// The ordinary agent surface.
    #[default]
    Agent,
    /// A server an operator deliberately started for themselves. Sessions also
    /// keep `agent`, because an operator may do everything an agent may.
    Operator,
}

impl McpSessionAuthority {
    pub(super) fn capabilities(self) -> BTreeSet<McpCapability> {
        match self {
            Self::Agent => BTreeSet::from([McpCapability::Agent]),
            Self::Operator => BTreeSet::from([McpCapability::Agent, McpCapability::Operator]),
        }
    }
}

/// Identity and trusted audit context derived by the accepting machine.
pub struct McpServerIdentity {
    pub process_machine_id: String,
    pub process_host_id: String,
    pub session_context: ToolSessionContext,
}

/// The caller identity a destination resolves a grant for.
///
/// The label is the caller's own claim, so it selects a row and grants
/// nothing. A remote-originated session that forwarded no label still resolves
/// through the callers file — under the accepting machine's own `machine_id`,
/// which is a caller the destination almost certainly has not granted anything
/// beyond the file default. Falling back to the caller's argv instead is the
/// escalation this exists to close [ORB-11052].
fn resolved_caller_machine_id(
    remote_caller_machine_id: Option<&str>,
    process_machine_id: &str,
) -> String {
    remote_caller_machine_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| process_machine_id.to_string(), ToOwned::to_owned)
}

/// Resolve the accepting machine and build its trusted session envelope.
///
/// `remote_caller_machine_id` is an opaque label forwarded by the SSH proxy,
/// not an authenticated principal. `SSH_CONNECTION`, when present, contributes
/// only the best-effort caller IP.
///
/// `policy` decides the session's capabilities. It is passed in rather than
/// derived here because the two are different questions: this function answers
/// "which machine is accepting, and how did the bytes arrive", and the policy
/// answers "what may this session do", which only `orbit mcp serve` resolves
/// against the destination's callers file.
pub fn mcp_server_identity(
    global_root: &Path,
    remote_caller_machine_id: Option<String>,
    policy: &SessionCapabilityPolicy,
) -> Result<McpServerIdentity, OrbitError> {
    let (process_machine_id, process_host_id) = local_identity(global_root)?;
    // Transport labeling keeps its own rule: a forwarded label marks an SSH
    // session for audit even where the policy is local, and a policy the
    // destination resolved from a callers file is by construction remote.
    let is_remote = remote_caller_machine_id.is_some() || policy.is_granted();
    // The policy already holds the identity the destination resolved, which
    // under a forced command is the one it wrote next to the authenticating
    // key rather than anything the caller forwarded. Re-deriving it from argv
    // here would quietly disagree with the grant recorded beside it.
    let caller_machine_id = policy.caller_machine_id().map_or_else(
        || resolved_caller_machine_id(remote_caller_machine_id.as_deref(), &process_machine_id),
        ToOwned::to_owned,
    );
    let mut session_context = ToolSessionContext {
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
    // No workspace has been resolved at session establishment, so this is the
    // session's unnarrowed ceiling; a `workspaces` row is re-evaluated per
    // call once the destination knows where the call lands.
    policy.stamp(&mut session_context, None);
    Ok(McpServerIdentity {
        process_machine_id,
        process_host_id,
        session_context,
    })
}

/// Resolve the caller identity and capability policy for one stdio
/// `orbit mcp serve` session.
///
/// This is the one entry point that consults the callers file. Whether it does
/// is decided here, by the destination — from a forced command it wrote itself
/// or, failing that, from its own environment. Never from the caller's argv,
/// which a caller can simply not write.
pub fn mcp_serve_session_policy(
    global_root: &Path,
    remote_caller_machine_id: Option<&str>,
    authority: McpSessionAuthority,
    acceptance: &SshAcceptance,
) -> Result<SessionCapabilityPolicy, OrbitError> {
    let Some(identity) = remote_caller_identity(global_root, remote_caller_machine_id, acceptance)?
    else {
        return Ok(SessionCapabilityPolicy::local(authority));
    };
    SessionCapabilityPolicy::resolve(global_root, authority, &identity)
}

/// Who this destination is serving, or `None` when the session is local.
///
/// Under a forced command the answer is the destination's own statement and
/// origination is settled: sshd ran a line out of this machine's
/// `authorized_keys`, so no environment check can be more authoritative than
/// that. Otherwise the Tier 1 observation decides, unchanged.
fn remote_caller_identity(
    global_root: &Path,
    remote_caller_machine_id: Option<&str>,
    acceptance: &SshAcceptance,
) -> Result<Option<RemoteCallerIdentity>, OrbitError> {
    let SshAcceptance::ForcedCommand {
        caller,
        acceptance_token,
    } = acceptance
    else {
        if !super::callers::remote_originated() {
            return Ok(None);
        }
        let (process_machine_id, _) = local_identity(global_root)?;
        let machine_id = resolved_caller_machine_id(remote_caller_machine_id, &process_machine_id);
        return Ok(Some(
            RemoteCallerIdentity::self_asserted(machine_id)
                .observing(ssh_auth::observe_authenticating_keys()),
        ));
    };
    if ssh_auth::ignored_original_command() {
        // Recorded because it was overridden, not because it was read: the
        // destination composed this argv, and the caller's string has no way
        // into the authority decision.
        tracing::info!(
            target: "orbit.mcp.callers",
            "serving a forced-command MCP session; the caller's SSH_ORIGINAL_COMMAND is ignored \
             entirely and contributes nothing to the requested authority"
        );
    }
    let Some(caller) = caller
        .as_deref()
        .map(str::trim)
        .filter(|caller| !caller.is_empty())
    else {
        return Err(OrbitError::UnauthorizedCaller(
            "SSH MCP acceptance did not name a caller; regenerate the authorized_keys line with \
             `orbit mcp callers authorize`"
                .to_string(),
        ));
    };
    validate_machine_id(caller).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "`orbit mcp serve --caller {caller}` is not a valid machine identity: {error}. This \
             argument comes from this machine's own authorized_keys; fix the forced command there"
        ))
    })?;
    let observed = ssh_auth::verify_ssh_acceptance(global_root, caller, acceptance_token)?;
    Ok(Some(RemoteCallerIdentity::key_bound(
        caller,
        Some(observed),
    )))
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
