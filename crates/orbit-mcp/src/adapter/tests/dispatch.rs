use std::sync::Arc;

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError,
    ToolSessionContext,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::super::OrbitToolServer;
use super::super::name_map::sanitize_tool_name;
use super::super::test_support::{EchoArrayHost, StubHost, tool_schema};

struct MissingPolicyHost;

struct CapabilityHost;

impl crate::McpHost for CapabilityHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        [
            ("demo.agent", McpCapability::Agent),
            ("demo.operator", McpCapability::Operator),
            ("demo.runner", McpCapability::Runner),
        ]
        .into_iter()
        .map(|(name, capability)| {
            let policy = McpToolPolicy::new(McpToolPlacement::Hub, [capability])
                .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
            McpToolDefinition::new(tool_schema(name), policy)
                .map_err(|error| OrbitError::InvalidInput(error.to_string()))
        })
        .collect()
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(json!({ "tool": name }))
    }
}

impl crate::McpHost for MissingPolicyHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Err(OrbitError::InvalidInput(
            "demo.unclassified is missing MCP policy".to_string(),
        ))
    }

    fn call_tool(
        &self,
        _name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(json!({ "must_not_execute": true }))
    }
}

#[test]
fn refresh_name_map_rejects_listing_collisions() {
    let host = Arc::new(StubHost {
        schemas: Vec::new(),
    });
    let server = OrbitToolServer::new(host);
    let schemas = vec![tool_schema("foo.bar"), tool_schema("foo_bar")];
    let err = server
        .refresh_name_map(&schemas)
        .expect_err("tools/list refresh must reject ambiguous advertised names");
    assert_eq!(err.advertised_name, "foo_bar");
}

#[tokio::test]
async fn missing_policy_is_excluded_and_rejected_before_dispatch() {
    let server = OrbitToolServer::new(Arc::new(MissingPolicyHost));
    assert!(
        server.combined_tool_schemas().is_err(),
        "invalid definitions fail the whole advertised surface closed"
    );

    let error = server
        .call_tool_request(CallToolRequestParams::new("demo_unclassified"))
        .await
        .expect_err("invalid definition source rejects before host dispatch");
    assert!(
        error
            .message
            .contains("invalid canonical MCP tool definitions")
    );
}

#[tokio::test]
async fn d2_context_membership_does_not_filter_tool_list_or_call() {
    let agent_server = OrbitToolServer::new(Arc::new(CapabilityHost));
    let agent_context = agent_server.session_context();
    assert!(agent_context.has_capability(McpCapability::Agent));
    assert!(!agent_context.has_capability(McpCapability::Operator));
    let agent_names = agent_server
        .combined_tool_schemas()
        .expect("agent tool list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    for expected in ["demo.agent", "demo.operator", "demo.runner"] {
        assert!(agent_names.iter().any(|name| name == expected));
    }

    let called = agent_server
        .call_tool_request(CallToolRequestParams::new("demo_operator"))
        .await
        .expect("D2 does not reject calls from policy metadata");
    assert_eq!(called.is_error, Some(false));

    let mut operator_context = ToolSessionContext::trusted_local(None, None, None);
    operator_context.effective_capabilities = [McpCapability::Operator].into_iter().collect();
    assert!(operator_context.has_capability(McpCapability::Operator));
    assert!(!operator_context.has_capability(McpCapability::Agent));
    let operator_server =
        OrbitToolServer::new_with_context(Arc::new(CapabilityHost), operator_context);
    let operator_names = operator_server
        .combined_tool_schemas()
        .expect("operator tool list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert_eq!(operator_names, agent_names);
}

#[tokio::test]
async fn call_tool_wraps_affected_array_results_for_strict_mcp_clients() {
    let affected_tools = [
        "orbit.task.list",
        "orbit.task.review_thread.list",
        "orbit.learning.list",
    ];
    let host = Arc::new(EchoArrayHost {
        schemas: affected_tools
            .iter()
            .map(|name| tool_schema(name))
            .collect(),
    });
    let server = OrbitToolServer::new(host);

    for canonical_name in affected_tools {
        let result = server
            .call_tool_request(CallToolRequestParams::new(sanitize_tool_name(
                canonical_name,
            )))
            .await
            .expect("MCP bridge call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(
            structured.is_object(),
            "{canonical_name} structuredContent must be object-shaped"
        );
        assert_eq!(
            structured.get("items"),
            Some(&json!([{ "tool": canonical_name }]))
        );

        let wire = serde_json::to_value(&result).expect("serialize CallToolResult");
        assert!(
            wire.get("structuredContent").is_some_and(Value::is_object),
            "{canonical_name} serialized structuredContent must satisfy record validators"
        );
    }
}

#[test]
fn canonical_name_translates_advertised_back_to_dotted() {
    let host = Arc::new(StubHost {
        schemas: vec![tool_schema("orbit.task.add")],
    });
    let server = OrbitToolServer::new(host);
    // Refreshes from host before resolving the advertised name.
    assert_eq!(
        server.canonical_name("orbit_task_add").unwrap(),
        "orbit.task.add"
    );
    // Repeated lookups preserve the same advertised-to-canonical mapping.
    assert_eq!(
        server.canonical_name("orbit_task_add").unwrap(),
        "orbit.task.add"
    );
}

#[test]
fn canonical_name_passes_through_unknown_or_legacy_dotted_names() {
    let host = Arc::new(StubHost {
        schemas: vec![tool_schema("orbit.task.add")],
    });
    let server = OrbitToolServer::new(host);
    // Legacy dotted name from an older client falls through unchanged so
    // the host's own tool-not-found handling still runs.
    assert_eq!(
        server.canonical_name("orbit.task.add").unwrap(),
        "orbit.task.add"
    );
    assert_eq!(
        server.canonical_name("totally.unknown").unwrap(),
        "totally.unknown"
    );
}

#[test]
fn canonical_name_rejects_sanitized_dispatch_collisions() {
    let host = Arc::new(StubHost {
        schemas: vec![tool_schema("foo.bar"), tool_schema("foo_bar")],
    });
    let server = OrbitToolServer::new(host);
    let err = server
        .canonical_name("foo_bar")
        .expect_err("dispatch must reject ambiguous advertised names");
    assert!(err.message.contains("duplicate advertised MCP tool name"));
}
