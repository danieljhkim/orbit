#![allow(missing_docs)]

use std::collections::BTreeSet;

use orbit_common::types::{McpCapability, McpTransport, ToolSessionContext};
use orbit_mcp::{McpCallContextResolver, McpRequestKind};
use serde_json::{Map, Value, json};

use super::super::owner_client::REMOTE_SESSION_META_KEY;
use super::super::transport::RemoteCallContextResolver;

#[test]
fn owner_calls_fail_closed_without_connector_metadata() {
    let trusted = trusted_agent_context();
    let error = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Tool {
                name: "orbit.task.show".to_string(),
            },
            &Map::new(),
        )
        .expect_err("missing connector metadata must fail closed");
    assert!(
        error
            .to_string()
            .contains("owner tool calls require connector-owned remote session metadata")
    );
}

/// The connector owns caller identity; the server owns grants. ORB-10727
/// [ADR-0358] also withdrew `runner` from the bridge and removed `leased_run`
/// from the session entirely, so neither can be smuggled in from the wire.
#[test]
fn connector_identity_is_retained_but_grants_are_server_owned() {
    let trusted = trusted_agent_context();
    let mut remote = remote_context();
    remote.effective_capabilities =
        BTreeSet::from([McpCapability::Operator, McpCapability::Runner]);
    let resolved = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Tool {
                name: "orbit.task.show".to_string(),
            },
            &metadata(remote),
        )
        .expect("valid connector metadata");

    assert_eq!(resolved.caller_machine_id.as_deref(), Some("hm_client"));
    assert_eq!(resolved.caller_host_id.as_deref(), Some("client"));
    assert_eq!(
        resolved.effective_capabilities,
        BTreeSet::from([McpCapability::Agent])
    );
}

/// A withdrawn `leased_run` key in connector metadata is inert: the field no
/// longer exists on the session, so it cannot reach audit correlation.
#[test]
fn withdrawn_leased_run_metadata_is_not_accepted() {
    let trusted = trusted_agent_context();
    let mut metadata = metadata(remote_context());
    let orbit = metadata
        .get_mut("orbit")
        .and_then(Value::as_object_mut)
        .expect("orbit metadata");
    let session = orbit
        .get_mut(REMOTE_SESSION_META_KEY)
        .and_then(Value::as_object_mut)
        .expect("session metadata");
    session.insert(
        "leased_run".to_string(),
        json!({"run_id": "forged-run", "lease_id": "forged-lease"}),
    );

    let resolved = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Tool {
                name: "orbit.task.show".to_string(),
            },
            &metadata,
        )
        .expect("unknown keys are ignored, not fatal");
    let serialized = serde_json::to_value(&resolved).expect("serialize resolved context");
    assert!(
        serialized.get("leased_run").is_none(),
        "no v1 code path accepts or emits leased_run: {serialized}"
    );
}

#[test]
fn connector_metadata_cannot_claim_owner_process_identity() {
    let trusted = trusted_agent_context();
    let mut remote = remote_context();
    remote.process_machine_id = Some("hm_owner".to_string());
    let error = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Tool {
                name: "orbit.task.show".to_string(),
            },
            &metadata(remote),
        )
        .expect_err("remote process identity is never caller-owned");
    assert!(error.to_string().contains("may not claim process identity"));
}

fn trusted_agent_context() -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_owner".to_string()),
        Some("owner".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
    context
}

fn remote_context() -> ToolSessionContext {
    ToolSessionContext {
        transport: Some(McpTransport::SshMcp),
        caller_machine_id: Some("hm_client".to_string()),
        caller_host_id: Some("client".to_string()),
        origin_session_id: Some("session-1".to_string()),
        mcp_call_id: Some("mcall-1".to_string()),
        ..ToolSessionContext::default()
    }
}

fn metadata(context: ToolSessionContext) -> Map<String, Value> {
    json!({
        "orbit": {
            (REMOTE_SESSION_META_KEY): context,
        }
    })
    .as_object()
    .expect("metadata object")
    .clone()
}
