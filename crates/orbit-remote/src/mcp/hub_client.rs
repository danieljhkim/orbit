//! Remote-owned hub client policy over the generic injected-stream MCP client.

use std::time::Duration;

use orbit_common::types::{
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

    pub(super) async fn close(&mut self, timeout: Duration) -> Result<(), OrbitError> {
        self.raw
            .close(timeout)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use orbit_common::types::{
        HostRegistration, McpLeasedRun, McpToolDefinition, McpToolPlacement, McpToolPolicy,
        SPOKE_REGISTRATION_SCHEMA_VERSION, SpokeRegistrationRequestV1, SpokeRegistrationResultV1,
        ToolSchema,
    };
    use orbit_mcp::{
        McpCustomRequestError, McpCustomRequestHandler, McpHost, McpServerComposition,
        McpServerMetadata, OrbitToolServer,
    };
    use rmcp::ServiceExt;
    use serde_json::json;
    use tokio::io::duplex;

    use super::*;
    use crate::mcp::contract::hub_schema_digest;
    use crate::mcp::transport::RemoteCallContextResolver;

    struct WireHost {
        definitions: Vec<McpToolDefinition>,
        instructions: String,
        calls: Mutex<Vec<ToolSessionContext>>,
        registrations: Mutex<Vec<(SpokeRegistrationRequestV1, ToolSessionContext)>>,
    }

    struct WireRegistrationHandler {
        host: Arc<WireHost>,
    }

    impl McpHost for WireHost {
        fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
            Ok(self.definitions.clone())
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

    impl McpCustomRequestHandler for WireRegistrationHandler {
        fn recognizes(&self, method: &str) -> bool {
            method == SPOKE_REGISTRATION_METHOD_V1
        }

        fn call(
            &self,
            _method: &str,
            params: Option<Value>,
            context: ToolSessionContext,
        ) -> Result<Value, McpCustomRequestError> {
            let params = params.ok_or_else(|| {
                McpCustomRequestError::invalid_params(
                    "private spoke registration requires parameters",
                )
            })?;
            let request = serde_json::from_value::<SpokeRegistrationRequestV1>(params)
                .map_err(|error| McpCustomRequestError::invalid_params(error.to_string()))?;
            self.host
                .registrations
                .lock()
                .expect("registrations")
                .push((request, context));
            serde_json::to_value(SpokeRegistrationResultV1::failed(
                None,
                None,
                Vec::new(),
                Vec::new(),
                "fixture_rejection",
                "definitive fixture rejection",
            ))
            .map_err(|error| McpCustomRequestError::internal(error.to_string()))
        }
    }

    fn wire_server(host: Arc<WireHost>, trusted: ToolSessionContext) -> OrbitToolServer {
        let composition = McpServerComposition::new()
            .with_call_context_resolver(Arc::new(RemoteCallContextResolver))
            .with_custom_request_handler(Arc::new(WireRegistrationHandler {
                host: Arc::clone(&host),
            }))
            .with_metadata(
                McpServerMetadata::default().with_instructions(host.instructions.clone()),
            );
        OrbitToolServer::new_with_context_and_composition(host, trusted, composition)
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
        let server = wire_server(host, trusted);
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
            leased_run: Some(McpLeasedRun {
                run_id: "spoofed-run".to_string(),
                lease_id: "spoofed-lease".to_string(),
            }),
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
        assert_eq!(result["leased_run"], Value::Null);
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
            .raw
            .custom_request(
                "orbit/private/guessed/v1",
                Some(json!({})),
                Map::new(),
                Duration::from_secs(1),
            )
            .await
            .expect_err("unknown private method");
        assert!(matches!(
            error,
            McpClientRequestError::Protocol { code, .. }
                if code == JSON_RPC_METHOD_NOT_FOUND
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
        let server = wire_server(host, ToolSessionContext::trusted_local(None, None, None));
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
