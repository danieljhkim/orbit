#![allow(missing_docs)]

use std::collections::BTreeSet;

use orbit_common::types::{
    McpCapability, McpLeasedRun, McpTransport, SPOKE_REGISTRATION_METHOD_V1, ToolSessionContext,
};
use orbit_mcp::{McpCallContextResolver, McpRequestKind};
use serde_json::{Map, Value, json};

use super::super::hub_client::REMOTE_SESSION_META_KEY;
use super::super::transport::RemoteCallContextResolver;

#[test]
fn hub_calls_fail_closed_without_connector_metadata() {
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
            .contains("hub tool calls require connector-owned remote session metadata")
    );
}

#[test]
fn connector_identity_is_retained_but_grants_and_lease_are_server_owned() {
    let trusted = trusted_agent_context();
    let mut remote = remote_context();
    remote.effective_capabilities =
        BTreeSet::from([McpCapability::Operator, McpCapability::Runner]);
    remote.leased_run = Some(McpLeasedRun {
        run_id: "forged-run".to_string(),
        lease_id: "forged-lease".to_string(),
    });
    let resolved = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Tool {
                name: "orbit.task.show".to_string(),
            },
            &metadata(remote),
        )
        .expect("valid connector metadata");

    assert_eq!(resolved.caller_machine_id.as_deref(), Some("hm_spoke"));
    assert_eq!(resolved.caller_host_id.as_deref(), Some("spoke"));
    assert_eq!(
        resolved.effective_capabilities,
        BTreeSet::from([McpCapability::Agent])
    );
    assert!(resolved.leased_run.is_none());
}

#[test]
fn connector_metadata_cannot_claim_hub_process_identity() {
    let trusted = trusted_agent_context();
    let mut remote = remote_context();
    remote.process_machine_id = Some("hm_hub".to_string());
    let error = RemoteCallContextResolver
        .resolve(
            &trusted,
            &McpRequestKind::Custom {
                method: SPOKE_REGISTRATION_METHOD_V1.to_string(),
            },
            &metadata(remote),
        )
        .expect_err("remote process identity is never caller-owned");
    assert!(error.to_string().contains("may not claim process identity"));
}

fn trusted_agent_context() -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([McpCapability::Agent]);
    context
}

fn remote_context() -> ToolSessionContext {
    ToolSessionContext {
        transport: Some(McpTransport::SshMcp),
        caller_machine_id: Some("hm_spoke".to_string()),
        caller_host_id: Some("spoke".to_string()),
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
