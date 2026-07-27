//! Connector-private MCP context and request composition.

use std::sync::Arc;

use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1,
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
                McpRequestKind::Custom { method }
                    if method == HUB_KNOWLEDGE_ALLOCATION_METHOD_V1 =>
                {
                    "private hub knowledge allocation requires connector-owned remote session metadata"
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

/// Typed custom handler for connector-private hub methods.
pub(super) struct PrivateHubRequestHandler {
    host: Arc<HubMcpHost>,
}

impl PrivateHubRequestHandler {
    pub(super) fn new(host: Arc<HubMcpHost>) -> Self {
        Self { host }
    }
}

impl McpCustomRequestHandler for PrivateHubRequestHandler {
    fn recognizes(&self, method: &str) -> bool {
        matches!(
            method,
            SPOKE_REGISTRATION_METHOD_V1 | HUB_KNOWLEDGE_ALLOCATION_METHOD_V1
        )
    }

    fn worker_label(&self) -> &'static str {
        "private hub request"
    }

    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        session_context: ToolSessionContext,
    ) -> Result<Value, McpCustomRequestError> {
        match method {
            SPOKE_REGISTRATION_METHOD_V1 => self.call_registration(params, session_context),
            HUB_KNOWLEDGE_ALLOCATION_METHOD_V1 => {
                self.call_knowledge_allocation(params, session_context)
            }
            _ => Err(McpCustomRequestError::MethodNotFound),
        }
    }
}

impl PrivateHubRequestHandler {
    fn call_registration(
        &self,
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

    fn call_knowledge_allocation(
        &self,
        params: Option<Value>,
        session_context: ToolSessionContext,
    ) -> Result<Value, McpCustomRequestError> {
        let params = params.ok_or_else(|| {
            McpCustomRequestError::invalid_params(
                "private hub knowledge allocation requires parameters",
            )
        })?;
        let request: HubKnowledgeAllocationRequestV1 =
            serde_json::from_value(params).map_err(|error| {
                McpCustomRequestError::invalid_params(format!(
                    "invalid private hub knowledge allocation payload: {error}"
                ))
            })?;
        request
            .validate()
            .map_err(|error| McpCustomRequestError::invalid_params(error.to_string()))?;
        let allocation = self
            .host
            .private_allocate_knowledge_id(request, session_context)
            .map_err(|error| match error {
                OrbitError::InvalidInput(message) => McpCustomRequestError::invalid_params(message),
                error => McpCustomRequestError::internal(error.to_string()),
            })?;
        serialize_allocation(allocation)
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

fn serialize_allocation(
    allocation: HubKnowledgeAllocationV1,
) -> Result<Value, McpCustomRequestError> {
    serde_json::to_value(allocation).map_err(|error| {
        McpCustomRequestError::internal(format!(
            "serialize private hub knowledge allocation result: {error}"
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
