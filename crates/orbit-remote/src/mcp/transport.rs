//! Connector-owned MCP call context for the owner endpoint.
//!
//! ORB-10727 [ADR-0358]: v1 negotiates no connector-private method. The
//! `orbit/private/register-spoke/v1` handler that used to live here is
//! withdrawn with the registration protocol — a client opens an owner route and
//! calls. What remains is the resolver that keeps connector-owned remote
//! identity separate from server-owned grants.

use orbit_common::types::{McpTransport, OrbitError, ToolSessionContext};
use orbit_mcp::{McpCallContextResolver, McpRequestKind};
use serde_json::{Map, Value};

use super::owner_client::REMOTE_SESSION_META_KEY;

/// Resolves connector-owned remote identity while retaining server-owned grants.
pub(super) struct RemoteCallContextResolver;

impl McpCallContextResolver for RemoteCallContextResolver {
    fn resolve(
        &self,
        trusted_context: &ToolSessionContext,
        _request: &McpRequestKind,
        transport_metadata: &Map<String, Value>,
    ) -> Result<ToolSessionContext, OrbitError> {
        let Some(remote) = remote_session_context_from_metadata(transport_metadata)? else {
            return Err(OrbitError::InvalidInput(
                "owner tool calls require connector-owned remote session metadata".to_string(),
            ));
        };
        if remote.transport != Some(McpTransport::SshMcp) {
            return Err(OrbitError::InvalidInput(
                "owner remote session metadata must declare ssh-mcp transport".to_string(),
            ));
        }
        if remote.caller_machine_id.is_none()
            || remote.caller_host_id.is_none()
            || remote.origin_session_id.is_none()
            || remote.mcp_call_id.is_none()
        {
            return Err(OrbitError::InvalidInput(
                "owner remote session metadata requires caller identity and call correlation"
                    .to_string(),
            ));
        }
        if remote.process_machine_id.is_some() || remote.process_host_id.is_some() {
            return Err(OrbitError::InvalidInput(
                "owner remote session metadata may not claim process identity".to_string(),
            ));
        }
        let mut remote = remote;
        remote.effective_capabilities = trusted_context.effective_capabilities.clone();
        Ok(remote)
    }
}

fn remote_session_context_from_metadata(
    metadata: &Map<String, Value>,
) -> Result<Option<ToolSessionContext>, OrbitError> {
    let Some(value) = metadata
        .get("orbit")
        .and_then(|orbit| orbit.get(REMOTE_SESSION_META_KEY))
    else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| {
            OrbitError::InvalidInput(format!("invalid owner remote session metadata: {error}"))
        })
}
