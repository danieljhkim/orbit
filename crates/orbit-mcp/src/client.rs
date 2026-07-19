//! Generic asynchronous MCP client for an injected byte-stream transport.

use std::time::Duration;

use orbit_common::types::{
    McpCapability, McpTransport, OrbitError, SPOKE_REGISTRATION_METHOD_V1,
    SpokeRegistrationRequestV1, SpokeRegistrationResultV1, ToolSessionContext,
};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientInfo, ClientRequest, CustomRequest, ErrorCode,
    Meta, ServerResult,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService, ServiceError};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::hub_contract::{
    CANONICAL_MCP_REGISTRY_REVISION, HubServerContractV1, MCP_CONTRACT_REVISION,
};

pub const REMOTE_SESSION_META_KEY: &str = "remote_session_context";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubClientExpectation {
    pub hub_machine_id: String,
    pub effective_capability: McpCapability,
    pub hub_schema_digest: String,
}

pub struct OrbitMcpClient {
    service: RunningService<RoleClient, ClientInfo>,
    contract: HubServerContractV1,
}

impl OrbitMcpClient {
    /// Initialize over caller-owned IO and fail before any tool call when the
    /// four frozen hub facts do not match.
    pub async fn connect<R, W>(
        read: R,
        write: W,
        expectation: &HubClientExpectation,
        initialize_timeout: Duration,
    ) -> Result<Self, OrbitError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let service = tokio::time::timeout(
            initialize_timeout,
            ClientInfo::default().serve((read, write)),
        )
        .await
        .map_err(|_| {
            OrbitError::HubUnavailable(format!(
                "MCP initialize exceeded {} ms",
                initialize_timeout.as_millis()
            ))
        })?
        .map_err(|error| OrbitError::HubUnavailable(format!("MCP initialize failed: {error}")))?;
        let contract = HubServerContractV1::parse_instructions(
            service
                .peer_info()
                .and_then(|info| info.instructions.as_deref()),
        )?;
        verify_contract(&contract, expectation)?;
        Ok(Self { service, contract })
    }

    pub fn contract(&self) -> &HubServerContractV1 {
        &self.contract
    }

    pub fn is_closed(&self) -> bool {
        self.service.is_closed()
    }

    /// Execute exactly once. Once rmcp accepts the request onto the initialized
    /// peer, every transport/protocol failure is outcome-unknown and retains
    /// the caller-generated `mcp_call_id`; this function never reconnects.
    pub async fn call_tool(
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
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let mut meta = Map::new();
        meta.insert(
            "orbit".to_string(),
            json!({ REMOTE_SESSION_META_KEY: context }),
        );
        let mut options = PeerRequestOptions::default();
        options.timeout = Some(request_timeout);
        options.meta = Some(Meta(meta));
        let handle = self
            .service
            .peer()
            .send_request_with_option(request, options)
            .await
            .map_err(|error| {
                OrbitError::HubUnavailable(format!(
                    "hub request was not handed to the initialized peer: {error}"
                ))
            })?;
        let response =
            handle
                .await_response()
                .await
                .map_err(|error| OrbitError::OutcomeUnknown {
                    mcp_call_id: mcp_call_id.clone(),
                    message: format!("hub response failed after request handoff: {error}"),
                })?;
        let ServerResult::CallToolResult(result) = response else {
            return Err(OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: "hub returned a non-tool result after request handoff".to_string(),
            });
        };
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
    pub async fn register_spoke(
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
        let request = ClientRequest::CustomRequest(CustomRequest::new(
            SPOKE_REGISTRATION_METHOD_V1,
            Some(params),
        ));
        let mut meta = Map::new();
        meta.insert(
            "orbit".to_string(),
            json!({ REMOTE_SESSION_META_KEY: context }),
        );
        let mut options = PeerRequestOptions::default();
        options.timeout = Some(request_timeout);
        options.meta = Some(Meta(meta));
        let handle = self
            .service
            .peer()
            .send_request_with_option(request, options)
            .await
            .map_err(|error| {
                OrbitError::HubUnavailable(format!(
                    "hub registration request was not handed to the initialized peer: {error}"
                ))
            })?;
        let response = match handle.await_response().await {
            Ok(response) => response,
            Err(ServiceError::McpError(error)) if error.code == ErrorCode::METHOD_NOT_FOUND => {
                return Err(OrbitError::HubNegotiation(format!(
                    "verified hub does not implement {SPOKE_REGISTRATION_METHOD_V1}"
                )));
            }
            Err(ServiceError::McpError(error)) if error.code == ErrorCode::INVALID_PARAMS => {
                return Err(OrbitError::RemoteTool {
                    code: "invalid_input".to_string(),
                    message: error.message.into_owned(),
                    payload: error.data.unwrap_or(Value::Null),
                });
            }
            Err(error) => {
                return Err(OrbitError::OutcomeUnknown {
                    mcp_call_id,
                    message: format!(
                        "hub registration response failed after request handoff: {error}"
                    ),
                });
            }
        };
        let ServerResult::CustomResult(result) = response else {
            return Err(OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: "hub returned a non-registration result after request handoff".to_string(),
            });
        };
        let result = result
            .result_as::<SpokeRegistrationResultV1>()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id: mcp_call_id.clone(),
                message: format!(
                    "hub returned a malformed registration result after request handoff: {error}"
                ),
            })?;
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

    pub async fn close(&mut self, timeout: Duration) -> Result<(), OrbitError> {
        self.service
            .close_with_timeout(timeout)
            .await
            .map_err(|error| OrbitError::Execution(format!("close MCP client: {error}")))?;
        Ok(())
    }
}

fn verify_contract(
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

pub fn validate_remote_call_context(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use orbit_common::types::{
        HostRegistration, McpToolDefinition, McpToolPlacement, McpToolPolicy,
        SPOKE_REGISTRATION_SCHEMA_VERSION, SpokeRegistrationRequestV1, SpokeRegistrationResultV1,
        ToolSchema,
    };
    use rmcp::ServiceExt;
    use serde_json::json;
    use tokio::io::duplex;

    use super::*;
    use crate::{
        CANONICAL_MCP_REGISTRY_REVISION, HubServerContractV1, MCP_CONTRACT_REVISION, McpHost,
        OrbitToolServer, hub_schema_digest,
    };

    struct WireHost {
        definitions: Vec<McpToolDefinition>,
        instructions: String,
        calls: Mutex<Vec<ToolSessionContext>>,
        registrations: Mutex<Vec<(SpokeRegistrationRequestV1, ToolSessionContext)>>,
    }

    impl McpHost for WireHost {
        fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
            Ok(self.definitions.clone())
        }

        fn private_server_instructions(&self) -> Option<String> {
            Some(self.instructions.clone())
        }

        fn accepts_remote_session_context(&self) -> bool {
            true
        }

        fn private_register_spoke(
            &self,
            request: SpokeRegistrationRequestV1,
            context: ToolSessionContext,
        ) -> Option<Result<SpokeRegistrationResultV1, OrbitError>> {
            self.registrations
                .lock()
                .expect("registrations")
                .push((request, context));
            Some(Ok(SpokeRegistrationResultV1::failed(
                None,
                None,
                Vec::new(),
                Vec::new(),
                "fixture_rejection",
                "definitive fixture rejection",
            )))
        }

        fn call_tool(
            &self,
            _name: &str,
            input: Value,
            context: ToolSessionContext,
        ) -> Result<Value, OrbitError> {
            self.calls.lock().expect("calls").push(context.clone());
            if input.get("fail").and_then(Value::as_bool) == Some(true) {
                return Err(OrbitError::InvalidInput("definitive failure".to_string()));
            }
            if input.get("delay").and_then(Value::as_bool) == Some(true) {
                std::thread::sleep(Duration::from_millis(100));
            }
            serde_json::to_value(context).map_err(|error| OrbitError::Execution(error.to_string()))
        }
    }

    fn wire_host() -> (Arc<WireHost>, HubClientExpectation) {
        let definitions = vec![
            McpToolDefinition::new(
                ToolSchema {
                    name: "orbit.task.show".to_string(),
                    description: "Show one task".to_string(),
                    parameters: Vec::new(),
                    builtin: true,
                },
                McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
            )
            .expect("definition"),
        ];
        let digest = hub_schema_digest(&definitions, McpCapability::Agent).expect("digest");
        let contract = HubServerContractV1 {
            contract_revision: MCP_CONTRACT_REVISION,
            canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
            hub_machine_id: "hm_hub".to_string(),
            effective_capability: McpCapability::Agent,
            hub_schema_digest: digest.clone(),
        };
        (
            Arc::new(WireHost {
                definitions,
                instructions: contract.instructions().expect("instructions"),
                calls: Mutex::new(Vec::new()),
                registrations: Mutex::new(Vec::new()),
            }),
            HubClientExpectation {
                hub_machine_id: "hm_hub".to_string(),
                effective_capability: McpCapability::Agent,
                hub_schema_digest: digest,
            },
        )
    }

    async fn wire_client(
        host: Arc<WireHost>,
        expectation: &HubClientExpectation,
    ) -> OrbitMcpClient {
        let (server_io, client_io) = duplex(64 * 1024);
        let mut trusted = ToolSessionContext::trusted_local(
            None,
            Some("hm_hub".to_string()),
            Some("hub".to_string()),
        );
        trusted.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
        let server = OrbitToolServer::new_with_context(host, trusted);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let (read, write) = tokio::io::split(client_io);
        OrbitMcpClient::connect(read, write, expectation, Duration::from_secs(1))
            .await
            .expect("client connect")
    }

    fn remote_context() -> ToolSessionContext {
        ToolSessionContext {
            workspace: Some("ws_orbit".to_string()),
            workspace_id: Some("ws_orbit".to_string()),
            caller_machine_id: Some("hm_spoke".to_string()),
            caller_host_id: Some("spoke".to_string()),
            transport: Some(McpTransport::SshMcp),
            effective_capabilities: BTreeSet::from([McpCapability::Agent]),
            origin_session_id: Some("session-1".to_string()),
            mcp_call_id: Some("mcall-1".to_string()),
            ..ToolSessionContext::default()
        }
    }

    fn registration_context() -> ToolSessionContext {
        ToolSessionContext {
            caller_machine_id: Some("hm_spoke".to_string()),
            caller_host_id: Some("spoke".to_string()),
            transport: Some(McpTransport::SshMcp),
            effective_capabilities: BTreeSet::from([McpCapability::Agent]),
            origin_session_id: Some("session-register".to_string()),
            mcp_call_id: Some("mcall-register".to_string()),
            ..ToolSessionContext::default()
        }
    }

    fn registration_request() -> SpokeRegistrationRequestV1 {
        SpokeRegistrationRequestV1 {
            schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
            identity: HostRegistration {
                machine_id: "hm_spoke".to_string(),
                host_id: "spoke".to_string(),
                labels: BTreeSet::new(),
            },
            presence: Vec::new(),
            profiles: Vec::new(),
        }
    }

    #[test]
    fn rejects_local_paths_and_process_claims_before_transport() {
        let mut context = ToolSessionContext {
            workspace: Some("/tmp/repo".to_string()),
            workspace_id: Some("/tmp/repo".to_string()),
            caller_machine_id: Some("hm_spoke".to_string()),
            caller_host_id: Some("spoke".to_string()),
            transport: Some(McpTransport::SshMcp),
            effective_capabilities: BTreeSet::from([McpCapability::Agent]),
            origin_session_id: Some("session-1".to_string()),
            mcp_call_id: Some("mcall-1".to_string()),
            ..ToolSessionContext::default()
        };
        assert!(validate_remote_call_context(&context, McpCapability::Agent).is_err());
        context.workspace = Some("ws_orbit".to_string());
        context.workspace_id = Some("ws_orbit".to_string());
        context.process_machine_id = Some("forged".to_string());
        assert!(validate_remote_call_context(&context, McpCapability::Agent).is_err());
    }

    #[tokio::test]
    async fn injected_duplex_negotiates_and_preserves_trusted_call_context() {
        let (host, expectation) = wire_host();
        let client = wire_client(Arc::clone(&host), &expectation).await;
        let result = client
            .call_tool(
                "orbit.task.show",
                json!({"workspace": "ws_orbit"}),
                &remote_context(),
                Duration::from_secs(1),
            )
            .await
            .expect("call");
        assert_eq!(result["caller_machine_id"], "hm_spoke");
        assert_eq!(result["mcp_call_id"], "mcall-1");
        assert_eq!(result["workspace_id"], "ws_orbit");
        assert_eq!(result["process_machine_id"], Value::Null);
        assert_eq!(host.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn structured_remote_error_is_definitive() {
        let (host, expectation) = wire_host();
        let client = wire_client(host, &expectation).await;
        let error = client
            .call_tool(
                "orbit.task.show",
                json!({"fail": true}),
                &remote_context(),
                Duration::from_secs(1),
            )
            .await
            .expect_err("remote error");
        assert!(matches!(
            error,
            OrbitError::RemoteTool { ref code, .. } if code == "invalid_input"
        ));
    }

    #[tokio::test]
    async fn private_registration_uses_typed_custom_request_and_remote_metadata() {
        let (host, expectation) = wire_host();
        let client = wire_client(Arc::clone(&host), &expectation).await;
        let result = client
            .register_spoke(
                &registration_request(),
                &registration_context(),
                Duration::from_secs(1),
            )
            .await
            .expect("definitive typed registration result");
        assert!(!result.complete);
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("fixture_rejection")
        );
        let registrations = host.registrations.lock().expect("registrations");
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].0.identity.machine_id, "hm_spoke");
        assert_eq!(
            registrations[0].1.mcp_call_id.as_deref(),
            Some("mcall-register")
        );
        assert_eq!(registrations[0].1.process_machine_id, None);
    }

    #[tokio::test]
    async fn unknown_private_method_is_method_not_found() {
        let (host, expectation) = wire_host();
        let client = wire_client(host, &expectation).await;
        let error = client
            .service
            .peer()
            .send_request(ClientRequest::CustomRequest(CustomRequest::new(
                "orbit/private/guessed/v1",
                Some(json!({})),
            )))
            .await
            .expect_err("unknown private method");
        assert!(matches!(
            error,
            ServiceError::McpError(ref data) if data.code == ErrorCode::METHOD_NOT_FOUND
        ));
    }

    #[tokio::test]
    async fn post_handoff_timeout_is_outcome_unknown_without_replay() {
        let (host, expectation) = wire_host();
        let client = wire_client(Arc::clone(&host), &expectation).await;
        let error = client
            .call_tool(
                "orbit.task.show",
                json!({"delay": true}),
                &remote_context(),
                Duration::from_millis(10),
            )
            .await
            .expect_err("timeout");
        assert!(matches!(
            error,
            OrbitError::OutcomeUnknown { ref mcp_call_id, .. } if mcp_call_id == "mcall-1"
        ));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(host.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn machine_pin_mismatch_fails_before_tool_dispatch() {
        let (host, mut expectation) = wire_host();
        expectation.hub_machine_id = "hm_wrong".to_string();
        let (server_io, client_io) = duplex(64 * 1024);
        let server = OrbitToolServer::new_with_context(
            host,
            ToolSessionContext::trusted_local(None, None, None),
        );
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let (read, write) = tokio::io::split(client_io);
        let error = OrbitMcpClient::connect(read, write, &expectation, Duration::from_secs(1))
            .await
            .err()
            .expect("negotiation failure");
        assert!(matches!(error, OrbitError::HubNegotiation(_)));
    }

    #[test]
    fn every_private_initialize_fact_is_verified_independently() {
        let expected = HubClientExpectation {
            hub_machine_id: "hm_hub".to_string(),
            effective_capability: McpCapability::Agent,
            hub_schema_digest: "digest".to_string(),
        };
        let baseline = HubServerContractV1 {
            contract_revision: MCP_CONTRACT_REVISION,
            canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
            hub_machine_id: expected.hub_machine_id.clone(),
            effective_capability: expected.effective_capability,
            hub_schema_digest: expected.hub_schema_digest.clone(),
        };
        assert!(verify_contract(&baseline, &expected).is_ok());

        let mut variants = Vec::new();
        let mut machine = baseline.clone();
        machine.hub_machine_id = "hm_other".to_string();
        variants.push(machine);
        let mut contract_revision = baseline.clone();
        contract_revision.contract_revision += 1;
        variants.push(contract_revision);
        let mut registry_revision = baseline.clone();
        registry_revision.canonical_registry_revision += 1;
        variants.push(registry_revision);
        let mut capability = baseline.clone();
        capability.effective_capability = McpCapability::Operator;
        variants.push(capability);
        let mut digest = baseline;
        digest.hub_schema_digest = "different".to_string();
        variants.push(digest);

        for actual in variants {
            assert!(matches!(
                verify_contract(&actual, &expected),
                Err(OrbitError::HubNegotiation(_))
            ));
        }
    }
}
