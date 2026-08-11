#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, McpTransport, OrbitError,
    ToolSchema, ToolSessionContext,
};
use orbit_mcp::{
    McpClientRequestError, McpHost, McpServerComposition, McpServerMetadata, OrbitToolServer,
    RawOrbitMcpClient,
};
use rmcp::ServiceExt;
use serde_json::{Map, Value, json};
use tokio::io::duplex;

use super::super::contract::{
    CANONICAL_MCP_REGISTRY_REVISION, MCP_CONTRACT_REVISION, OwnerServerContractV1,
    owner_schema_digest,
};
use super::super::owner_client::{
    OrbitMcpClient, OwnerClientExpectation, validate_remote_call_context, verify_contract,
};
use super::super::transport::RemoteCallContextResolver;

const JSON_RPC_METHOD_NOT_FOUND: i32 = -32601;

struct WireHost {
    definitions: Vec<McpToolDefinition>,
    instructions: String,
    calls: Mutex<Vec<ToolSessionContext>>,
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

fn wire_server(host: Arc<WireHost>, trusted: ToolSessionContext) -> OrbitToolServer {
    // ORB-10727 [ADR-0358]: no custom request handler is registered, because
    // v1 has no connector-private method for one to serve.
    let composition = McpServerComposition::new()
        .with_call_context_resolver(Arc::new(RemoteCallContextResolver))
        .with_metadata(McpServerMetadata::default().with_instructions(host.instructions.clone()));
    OrbitToolServer::new_with_context_and_composition(host, trusted, composition)
}

fn wire_host() -> (Arc<WireHost>, OwnerClientExpectation) {
    let definitions = vec![
        McpToolDefinition::new(
            ToolSchema {
                name: "orbit.task.show".to_string(),
                description: "Show one task".to_string(),
                parameters: Vec::new(),
                builtin: true,
            },
            McpToolPolicy::agent_and_operator(McpToolPlacement::Owner),
        )
        .expect("definition"),
    ];
    let digest = owner_schema_digest(&definitions, McpCapability::Agent).expect("digest");
    let contract = OwnerServerContractV1 {
        contract_revision: MCP_CONTRACT_REVISION,
        canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
        owner_machine_id: "hm_owner".to_string(),
        effective_capability: McpCapability::Agent,
        owner_schema_digest: digest.clone(),
    };
    (
        Arc::new(WireHost {
            definitions,
            instructions: contract.instructions().expect("instructions"),
            calls: Mutex::new(Vec::new()),
        }),
        OwnerClientExpectation {
            owner_machine_id: "hm_owner".to_string(),
            effective_capability: McpCapability::Agent,
            owner_schema_digest: digest,
        },
    )
}

async fn wire_client(host: Arc<WireHost>, expectation: &OwnerClientExpectation) -> OrbitMcpClient {
    let (server_io, client_io) = duplex(64 * 1024);
    let mut trusted = ToolSessionContext::trusted_local(
        None,
        Some("hm_owner".to_string()),
        Some("owner".to_string()),
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
        ..ToolSessionContext::default()
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
    // ORB-10727 [ADR-0358]: `leased_run` is not a field the wire can carry.
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

/// ORB-10727 [ADR-0358]: the owner endpoint composes no custom request handler,
/// so the withdrawn registration method is not merely unimplemented — it is
/// indistinguishable from any other unknown method.
#[tokio::test]
async fn the_withdrawn_registration_method_is_not_negotiated() {
    let (host, _expectation) = wire_host();
    let (server_io, client_io) = duplex(64 * 1024);
    let server = wire_server(host, ToolSessionContext::trusted_local(None, None, None));
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let (read, write) = tokio::io::split(client_io);
    let raw = RawOrbitMcpClient::connect(read, write, Duration::from_secs(1))
        .await
        .expect("raw client connect");
    let error = raw
        .custom_request(
            "orbit/private/register-spoke/v1",
            Some(json!({"schema_version": 1})),
            Map::new(),
            Duration::from_secs(1),
        )
        .await
        .expect_err("withdrawn private method");
    assert!(matches!(
        error,
        McpClientRequestError::Protocol { code, .. }
            if code == JSON_RPC_METHOD_NOT_FOUND
    ));
}

#[tokio::test]
async fn unknown_private_method_is_method_not_found() {
    let (host, _expectation) = wire_host();
    let (server_io, client_io) = duplex(64 * 1024);
    let server = wire_server(host, ToolSessionContext::trusted_local(None, None, None));
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let (read, write) = tokio::io::split(client_io);
    let raw = RawOrbitMcpClient::connect(read, write, Duration::from_secs(1))
        .await
        .expect("raw client connect");
    let error = raw
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
    expectation.owner_machine_id = "hm_wrong".to_string();
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
    assert!(matches!(error, OrbitError::OwnerNegotiation(_)));
}

#[test]
fn every_initialize_fact_is_verified_independently() {
    let expected = OwnerClientExpectation {
        owner_machine_id: "hm_owner".to_string(),
        effective_capability: McpCapability::Agent,
        owner_schema_digest: "digest".to_string(),
    };
    let baseline = OwnerServerContractV1 {
        contract_revision: MCP_CONTRACT_REVISION,
        canonical_registry_revision: CANONICAL_MCP_REGISTRY_REVISION,
        owner_machine_id: expected.owner_machine_id.clone(),
        effective_capability: expected.effective_capability,
        owner_schema_digest: expected.owner_schema_digest.clone(),
    };
    assert!(verify_contract(&baseline, &expected).is_ok());

    let mut variants = Vec::new();
    let mut machine = baseline.clone();
    machine.owner_machine_id = "hm_other".to_string();
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
    digest.owner_schema_digest = "different".to_string();
    variants.push(digest);

    for actual in variants {
        assert!(matches!(
            verify_contract(&actual, &expected),
            Err(OrbitError::OwnerNegotiation(_))
        ));
    }
}
