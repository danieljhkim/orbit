use std::collections::BTreeSet;

use orbit_types::tool::{McpCapability, McpTransport};

use super::super::callers::SessionCapabilityPolicy;
use super::super::identity::{
    McpSessionAuthority, local_identity, mcp_server_identity, ssh_caller_ip,
};

#[test]
fn absent_host_identity_uses_explicit_local_fallbacks() {
    let root = tempfile::tempdir().expect("global root");

    let (machine_id, host_id) = local_identity(root.path()).expect("local identity");

    assert_eq!(machine_id, "host/local");
    assert!(!host_id.is_empty());
}

#[test]
fn present_host_identity_is_server_derived() {
    let root = tempfile::tempdir().expect("global root");
    let outcome = orbit_registry::ensure_host_identity(root.path(), || {
        Ok(orbit_registry::NewHostIdentity {
            host_id: "server-host".to_string(),
            task_prefix: "SV".to_string(),
        })
    })
    .expect("host identity");
    let identity = outcome.identity();

    let actual = local_identity(root.path()).expect("local identity");

    assert_eq!(
        actual,
        (identity.machine_id.clone(), identity.host_id.clone())
    );
}

#[test]
fn ssh_connection_contributes_only_the_observed_caller_ip() {
    let caller_ip = ssh_caller_ip("192.0.2.8 43100 198.51.100.2 22");

    assert_eq!(caller_ip.as_deref(), Some("192.0.2.8"));
}

#[test]
fn remote_context_keeps_identity_and_transport_concepts_separate() {
    let root = tempfile::tempdir().expect("global root");
    let identity = mcp_server_identity(
        root.path(),
        Some("hm_caller".to_string()),
        &SessionCapabilityPolicy::local(McpSessionAuthority::Agent),
    )
    .expect("MCP server identity");

    assert_eq!(
        identity.session_context.caller_machine_id.as_deref(),
        Some("hm_caller")
    );
    assert_eq!(
        identity.session_context.process_machine_id.as_deref(),
        Some(identity.process_machine_id.as_str())
    );
    assert_eq!(
        identity.session_context.transport,
        Some(McpTransport::SshMcp)
    );
}

#[test]
fn a_default_server_serves_agent_sessions_only() {
    let root = tempfile::tempdir().expect("global root");

    let identity = mcp_server_identity(
        root.path(),
        None,
        &SessionCapabilityPolicy::local(McpSessionAuthority::Agent),
    )
    .expect("MCP server identity");

    assert_eq!(
        identity.session_context.effective_capabilities,
        BTreeSet::from([McpCapability::Agent]),
        "an agent server must never stamp operator authority onto its sessions"
    );
}

#[test]
fn an_operator_server_grants_the_capability_governed_tools_require() {
    let root = tempfile::tempdir().expect("global root");

    let identity = mcp_server_identity(
        root.path(),
        None,
        &SessionCapabilityPolicy::local(McpSessionAuthority::Operator),
    )
    .expect("MCP server identity");

    assert_eq!(
        identity.session_context.effective_capabilities,
        BTreeSet::from([McpCapability::Agent, McpCapability::Operator])
    );
}
