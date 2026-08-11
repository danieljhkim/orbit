//! Remote-owned owner-route client policy over the generic injected-stream MCP
//! client.

use std::time::Duration;

use orbit_common::types::{McpCapability, McpTransport, OrbitError, ToolSessionContext};
use orbit_mcp::{McpClientRequestError, RawOrbitMcpClient};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use super::contract::{
    CANONICAL_MCP_REGISTRY_REVISION, MCP_CONTRACT_REVISION, OwnerServerContractV1,
};

pub(super) const REMOTE_SESSION_META_KEY: &str = "remote_session_context";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnerClientExpectation {
    pub(super) owner_machine_id: String,
    pub(super) effective_capability: McpCapability,
    pub(super) owner_schema_digest: String,
}

pub(super) struct OrbitMcpClient {
    raw: RawOrbitMcpClient,
    contract: OwnerServerContractV1,
}

impl OrbitMcpClient {
    /// Initialize over caller-owned IO and fail before any tool call when the
    /// four frozen owner facts do not match.
    pub(super) async fn connect<R, W>(
        read: R,
        write: W,
        expectation: &OwnerClientExpectation,
        initialize_timeout: Duration,
    ) -> Result<Self, OrbitError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let raw = RawOrbitMcpClient::connect(read, write, initialize_timeout)
            .await
            .map_err(|error| OrbitError::OwnerUnavailable(error.to_string()))?;
        let contract = OwnerServerContractV1::parse_instructions(
            raw.initialization().instructions.as_deref(),
        )?;
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
                McpClientRequestError::PreHandoff { message } => OrbitError::OwnerUnavailable(
                    format!("owner request was not handed to the initialized peer: {message}"),
                ),
                McpClientRequestError::UnexpectedResponse { .. } => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: "owner returned a non-tool result after request handoff".to_string(),
                },
                error => OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!("owner response failed after request handoff: {error}"),
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
                .unwrap_or("owner tool returned an error")
                .to_string();
            return Err(OrbitError::RemoteTool {
                code,
                message,
                payload: structured,
            });
        }
        Ok(structured)
    }

    pub(super) async fn close(&mut self, timeout: Duration) -> Result<(), OrbitError> {
        self.raw
            .close(timeout)
            .await
            .map_err(|error| OrbitError::Execution(format!("close MCP client: {error}")))?;
        Ok(())
    }
}

/// Verify the frozen owner contract after transport initialization.
pub(super) fn verify_contract(
    actual: &OwnerServerContractV1,
    expected: &OwnerClientExpectation,
) -> Result<(), OrbitError> {
    let mut mismatches = Vec::new();
    if actual.owner_machine_id != expected.owner_machine_id {
        mismatches.push(format!(
            "owner machine_id expected '{}' but received '{}'",
            expected.owner_machine_id, actual.owner_machine_id
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
    if actual.owner_schema_digest != expected.owner_schema_digest {
        mismatches.push(format!(
            "owner schema digest expected '{}' but received '{}'",
            expected.owner_schema_digest, actual.owner_schema_digest
        ));
    }
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(OrbitError::OwnerNegotiation(mismatches.join("; ")))
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
            "remote MCP context must not claim the owner process identity".to_string(),
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
