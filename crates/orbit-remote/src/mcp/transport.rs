//! Connector-private MCP context and registration composition.

use std::sync::Arc;

use orbit_common::types::{
    McpTransport, OrbitError, SPOKE_REGISTRATION_METHOD_V1, SpokeRegistrationRequestV1,
    SpokeRegistrationResultV1, ToolSessionContext,
};
use orbit_mcp::{
    McpCallContextResolver, McpCustomRequestError, McpCustomRequestHandler, McpRequestKind,
};
use serde_json::{Map, Value};

use super::hub::HubMcpHost;
use super::hub_client::REMOTE_SESSION_META_KEY;

/// Resolves connector-owned remote identity while retaining server-owned grants.
pub(super) struct RemoteCallContextResolver;

impl McpCallContextResolver for RemoteCallContextResolver {
    fn resolve(
        &self,
        trusted_context: &ToolSessionContext,
        request: &McpRequestKind,
        transport_metadata: &Map<String, Value>,
    ) -> Result<ToolSessionContext, OrbitError> {
        let Some(remote) = remote_session_context_from_metadata(transport_metadata)? else {
            let message = match request {
                McpRequestKind::Custom { method } if method == SPOKE_REGISTRATION_METHOD_V1 => {
                    "private spoke registration requires connector-owned remote session metadata"
                }
                _ => "hub tool calls require connector-owned remote session metadata",
            };
            return Err(OrbitError::InvalidInput(message.to_string()));
        };
        if remote.transport != Some(McpTransport::SshMcp) {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata must declare ssh-mcp transport".to_string(),
            ));
        }
        if remote.caller_machine_id.is_none()
            || remote.caller_host_id.is_none()
            || remote.origin_session_id.is_none()
            || remote.mcp_call_id.is_none()
        {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata requires caller identity and call correlation"
                    .to_string(),
            ));
        }
        if remote.process_machine_id.is_some() || remote.process_host_id.is_some() {
            return Err(OrbitError::InvalidInput(
                "hub remote session metadata may not claim process identity".to_string(),
            ));
        }
        let mut remote = remote;
        remote.effective_capabilities = trusted_context.effective_capabilities.clone();
        remote.leased_run = None;
        Ok(remote)
    }
}

/// Typed custom handler for the connector-private spoke bootstrap method.
pub(super) struct SpokeRegistrationHandler {
    host: Arc<HubMcpHost>,
}

impl SpokeRegistrationHandler {
    pub(super) fn new(host: Arc<HubMcpHost>) -> Self {
        Self { host }
    }
}

impl McpCustomRequestHandler for SpokeRegistrationHandler {
    fn recognizes(&self, method: &str) -> bool {
        method == SPOKE_REGISTRATION_METHOD_V1
    }

    fn worker_label(&self) -> &'static str {
        "private spoke registration"
    }

    fn call(
        &self,
        _method: &str,
        params: Option<Value>,
        session_context: ToolSessionContext,
    ) -> Result<Value, McpCustomRequestError> {
        let params = params.ok_or_else(|| {
            McpCustomRequestError::invalid_params("private spoke registration requires parameters")
        })?;
        let registration: SpokeRegistrationRequestV1 =
            serde_json::from_value(params).map_err(|error| {
                McpCustomRequestError::invalid_params(format!(
                    "invalid private spoke registration payload: {error}"
                ))
            })?;
        if let Err(error) = registration.validate() {
            return serialize_registration_result(SpokeRegistrationResultV1::rejected(&error));
        }
        let result = match self
            .host
            .private_register_spoke(registration, session_context)
        {
            Ok(result) => result,
            Err(error) => SpokeRegistrationResultV1::rejected(&error),
        };
        result.validate().map_err(|error| {
            McpCustomRequestError::internal(format!(
                "invalid private spoke registration result: {error}"
            ))
        })?;
        serialize_registration_result(result)
    }
}

fn serialize_registration_result(
    result: SpokeRegistrationResultV1,
) -> Result<Value, McpCustomRequestError> {
    serde_json::to_value(result).map_err(|error| {
        McpCustomRequestError::internal(format!(
            "serialize private spoke registration result: {error}"
        ))
    })
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
            OrbitError::InvalidInput(format!("invalid hub remote session metadata: {error}"))
        })
}
