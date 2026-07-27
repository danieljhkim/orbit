//! Remote-owned hub client policy over the generic injected-stream MCP client.

use std::time::Duration;

use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1,
    McpCapability, McpTransport, OrbitError, SPOKE_REGISTRATION_METHOD_V1,
    SpokeRegistrationRequestV1, SpokeRegistrationResultV1, ToolSessionContext,
};
use orbit_mcp::{McpClientRequestError, RawOrbitMcpClient};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use super::contract::{
    CANONICAL_MCP_REGISTRY_REVISION, HubServerContractV1, MCP_CONTRACT_REVISION,
};

const JSON_RPC_METHOD_NOT_FOUND: i32 = -32601;
const JSON_RPC_INVALID_PARAMS: i32 = -32602;

pub(super) const REMOTE_SESSION_META_KEY: &str = "remote_session_context";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HubClientExpectation {
    pub(super) hub_machine_id: String,
    pub(super) effective_capability: McpCapability,
    pub(super) hub_schema_digest: String,
}

pub(super) struct OrbitMcpClient {
    raw: RawOrbitMcpClient,
    contract: HubServerContractV1,
}

impl OrbitMcpClient {
    /// Initialize over caller-owned IO and fail before any tool call when the
    /// four frozen hub facts do not match.
    pub(super) async fn connect<R, W>(
        read: R,
        write: W,
        expectation: &HubClientExpectation,
        initialize_timeout: Duration,
    ) -> Result<Self, OrbitError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let raw = RawOrbitMcpClient::connect(read, write, initialize_timeout)
            .await
            .map_err(|error| OrbitError::HubUnavailable(error.to_string()))?;
        let contract =
            HubServerContractV1::parse_instructions(raw.initialization().instructions.as_deref())?;
        verify_contract(&contract, expectation)?;
        Ok(Self { raw, contract })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.raw.is_closed()
    }

    /// Execute exactly once. Once the generic MCP client accepts the request
    /// onto the initialized peer, every transport/protocol failure is
    /// outcome-unknown and retains the caller-generated `mcp_call_id`; this
    /// function never reconnects.
    pub(super) async fn call_tool(
        &self,
        name: &str,
        input: Value,
        context: &ToolSessionContext,
        request_timeout: Duration,
    ) -> Result<Value, OrbitError> {
        validate_remote_call_context(context, self.contract.effective_capability)?;
        let mcp_call_id = context
            .mcp_call_id
            .as_deref()
            .ok_or_else(|| OrbitError::InvalidInput("remote MCP call requires mcp_call_id".into()))?
            .to_string();
        let arguments = input.as_object().cloned().ok_or_else(|| {
            OrbitError::InvalidInput("remote MCP tool input must be a JSON object".to_string())
        })?;
        let mut meta = Map::new();
        meta.insert(
            "orbit".to_string(),
            json!({ REMOTE_SESSION_META_KEY: context }),
        );
        let result = self
            .raw
            .call_tool(name, arguments, meta, request_timeout)
            .await
            .map_err(|error| match error {
                McpClientRequestError::PreHandoff { message } => OrbitError::HubUnavailable(
                    format!("hub request was not handed to the initialized peer: {message}"),
                ),
                McpClientRequestError::UnexpectedResponse { .. } => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: "hub returned a non-tool result after request handoff".to_string(),
                },
                error => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!("hub response failed after request handoff: {error}"),
                },
            })?;
        let structured = result.structured_content.unwrap_or(Value::Null);
        if result.is_error == Some(true) {
            let code = structured
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("remote_tool_error")
                .to_string();
            let message = structured
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("hub tool returned an error")
                .to_string();
            return Err(OrbitError::RemoteTool {
                code,
                message,
                payload: structured,
            });
        }
        Ok(structured)
    }

    /// Execute the single connector-private registration request exactly once.
    ///
    /// The initialize contract has already been verified by [`Self::connect`].
    /// A typed partial result is definitive; a transport/protocol loss after
    /// handoff is outcome-unknown and is never replayed here.
    pub(super) async fn register_spoke(
        &self,
        request: &SpokeRegistrationRequestV1,
        context: &ToolSessionContext,
        request_timeout: Duration,
    ) -> Result<SpokeRegistrationResultV1, OrbitError> {
        request.validate()?;
        validate_remote_call_context(context, self.contract.effective_capability)?;
        if context.workspace.is_some() || context.workspace_id.is_some() {
            return Err(OrbitError::InvalidInput(
                "private spoke registration is global and must not carry a workspace selector"
                    .to_string(),
            ));
        }
        let mcp_call_id = context
            .mcp_call_id
            .as_deref()
            .ok_or_else(|| OrbitError::InvalidInput("remote MCP call requires mcp_call_id".into()))?
            .to_string();
        let params = serde_json::to_value(request).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "serialize private spoke registration request: {error}"
            ))
        })?;
        if !params.is_object() {
            return Err(OrbitError::InvalidInput(
                "private spoke registration request must serialize as a JSON object".to_string(),
            ));
        }
        let mut meta = Map::new();
        meta.insert(
            "orbit".to_string(),
            json!({ REMOTE_SESSION_META_KEY: context }),
        );
        let response = self
            .raw
            .custom_request(
                SPOKE_REGISTRATION_METHOD_V1,
                Some(params),
                meta,
                request_timeout,
            )
            .await
            .map_err(|error| match error {
                McpClientRequestError::PreHandoff { message } => {
                    OrbitError::HubUnavailable(format!(
                        "hub registration request was not handed to the initialized peer: {message}"
                    ))
                }
                McpClientRequestError::Protocol { code, .. }
                    if code == JSON_RPC_METHOD_NOT_FOUND =>
                {
                    OrbitError::HubNegotiation(format!(
                        "verified hub does not implement {SPOKE_REGISTRATION_METHOD_V1}"
                    ))
                }
                McpClientRequestError::Protocol {
                    code,
                    message,
                    data,
                } if code == JSON_RPC_INVALID_PARAMS => OrbitError::RemoteTool {
                    code: "invalid_input".to_string(),
                    message,
                    payload: data.unwrap_or(Value::Null),
                },
                McpClientRequestError::UnexpectedResponse { .. } => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: "hub returned a non-registration result after request handoff"
                        .to_string(),
                },
                error => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!(
                        "hub registration response failed after request handoff: {error}"
                    ),
                },
            })?;
        let result = serde_json::from_value::<SpokeRegistrationResultV1>(response).map_err(
            |error| OrbitError::OutcomeUnknown {
                mcp_call_id: mcp_call_id.clone(),
                message: format!(
                    "hub returned a malformed registration result after request handoff: {error}"
                ),
            },
        )?;
        result
            .validate()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!(
                    "hub returned an invalid registration result after request handoff: {error}"
                ),
            })?;
        Ok(result)
    }

    /// Execute the connector-private hub knowledge allocation exactly once.
    ///
    /// The caller retains `mcp_call_id` on every post-handoff failure and must
    /// use the allocation lookup seam to resolve an outcome-unknown response;
    /// this client never replays the request automatically.
    pub(super) async fn allocate_knowledge_id(
        &self,
        request: &HubKnowledgeAllocationRequestV1,
        context: &ToolSessionContext,
        request_timeout: Duration,
    ) -> Result<HubKnowledgeAllocationV1, OrbitError> {
        request.validate()?;
        validate_remote_call_context(context, self.contract.effective_capability)?;
        if context.workspace_id.as_deref() != Some(request.workspace_id.as_str()) {
            return Err(OrbitError::InvalidInput(
                "private hub knowledge allocation workspace must exactly match the trusted remote context"
                    .to_string(),
            ));
        }
        let mcp_call_id = context
            .mcp_call_id
            .as_deref()
            .ok_or_else(|| OrbitError::InvalidInput("remote MCP call requires mcp_call_id".into()))?
            .to_string();
        let params = serde_json::to_value(request).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "serialize private hub knowledge allocation request: {error}"
            ))
        })?;
        let mut meta = Map::new();
        meta.insert(
            "orbit".to_string(),
            json!({ REMOTE_SESSION_META_KEY: context }),
        );
        let response = self
            .raw
            .custom_request(
                HUB_KNOWLEDGE_ALLOCATION_METHOD_V1,
                Some(params),
                meta,
                request_timeout,
            )
            .await
            .map_err(|error| match error {
                McpClientRequestError::PreHandoff { message } => {
                    OrbitError::HubUnavailable(format!(
                        "hub allocation request was not handed to the initialized peer: {message}"
                    ))
                }
                McpClientRequestError::Protocol { code, .. }
                    if code == JSON_RPC_METHOD_NOT_FOUND =>
                {
                    OrbitError::HubNegotiation(format!(
                        "verified hub does not implement {HUB_KNOWLEDGE_ALLOCATION_METHOD_V1}"
                    ))
                }
                McpClientRequestError::Protocol {
                    code,
                    message,
                    data,
                } if code == JSON_RPC_INVALID_PARAMS => OrbitError::RemoteTool {
                    code: "invalid_input".to_string(),
                    message,
                    payload: data.unwrap_or(Value::Null),
                },
                McpClientRequestError::UnexpectedResponse { .. } => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: "hub returned a non-allocation result after request handoff"
                        .to_string(),
                },
                error => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!(
                        "hub allocation response failed after request handoff: {error}"
                    ),
                },
            })?;
        let allocation =
            serde_json::from_value::<HubKnowledgeAllocationV1>(response).map_err(|error| {
                OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!(
                        "hub returned a malformed allocation result after request handoff: {error}"
                    ),
                }
            })?;
        allocation
            .validate()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!(
                    "hub returned an invalid allocation result after request handoff: {error}"
                ),
            })?;
        Ok(allocation)
    }

    pub(super) async fn close(&mut self, timeout: Duration) -> Result<(), OrbitError> {
        self.raw
            .close(timeout)
            .await
            .map_err(|error| OrbitError::Execution(format!("close MCP client: {error}")))?;
        Ok(())
    }
}

/// Verify the frozen hub contract after transport initialization.
pub(super) fn verify_contract(
    actual: &HubServerContractV1,
    expected: &HubClientExpectation,
) -> Result<(), OrbitError> {
    let mut mismatches = Vec::new();
    if actual.hub_machine_id != expected.hub_machine_id {
        mismatches.push(format!(
            "hub machine_id expected '{}' but received '{}'",
            expected.hub_machine_id, actual.hub_machine_id
        ));
    }
    if actual.contract_revision != MCP_CONTRACT_REVISION {
        mismatches.push(format!(
            "contract revision expected {MCP_CONTRACT_REVISION} but received {}",
            actual.contract_revision
        ));
    }
    if actual.canonical_registry_revision != CANONICAL_MCP_REGISTRY_REVISION {
        mismatches.push(format!(
            "canonical registry revision expected {CANONICAL_MCP_REGISTRY_REVISION} but received {}",
            actual.canonical_registry_revision
        ));
    }
    if actual.effective_capability != expected.effective_capability {
        mismatches.push(format!(
            "effective capability expected '{}' but received '{}'",
            expected.effective_capability, actual.effective_capability
        ));
    }
    if actual.hub_schema_digest != expected.hub_schema_digest {
        mismatches.push(format!(
            "hub schema digest expected '{}' but received '{}'",
            expected.hub_schema_digest, actual.hub_schema_digest
        ));
    }
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(OrbitError::HubNegotiation(mismatches.join("; ")))
    }
}

pub(super) fn validate_remote_call_context(
    context: &ToolSessionContext,
    capability: McpCapability,
) -> Result<(), OrbitError> {
    match (
        context.workspace.as_deref(),
        context.workspace_id.as_deref(),
    ) {
        (None, None) => {}
        (Some(workspace), Some(workspace_id))
            if workspace == workspace_id
                && !workspace_id.contains('/')
                && workspace_id != "."
                && workspace_id != ".." => {}
        _ => {
            return Err(OrbitError::InvalidInput(
                "remote MCP workspace must be absent for a global call or equal one stable logical workspace_id; paths are forbidden"
                    .to_string(),
            ));
        }
    }
    if context.caller_machine_id.is_none() || context.caller_host_id.is_none() {
        return Err(OrbitError::InvalidInput(
            "remote MCP call requires caller machine_id and host_id".to_string(),
        ));
    }
    if context.transport != Some(McpTransport::SshMcp) {
        return Err(OrbitError::InvalidInput(
            "remote MCP call transport must be ssh-mcp".to_string(),
        ));
    }
    if context.process_machine_id.is_some() || context.process_host_id.is_some() {
        return Err(OrbitError::InvalidInput(
            "remote MCP context must not claim the hub process identity".to_string(),
        ));
    }
    if context.origin_session_id.is_none() || context.mcp_call_id.is_none() {
        return Err(OrbitError::InvalidInput(
            "remote MCP call requires origin_session_id and mcp_call_id".to_string(),
        ));
    }
    if context.effective_capabilities.len() != 1
        || !context.effective_capabilities.contains(&capability)
    {
        return Err(OrbitError::InvalidInput(format!(
            "remote MCP effective capability must be exactly '{capability}'"
        )));
    }
    Ok(())
}
