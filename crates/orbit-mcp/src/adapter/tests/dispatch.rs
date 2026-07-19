use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

struct RemoteMetadataHost {
    called: AtomicBool,
}

impl crate::McpHost for RemoteMetadataHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(vec![
            McpToolDefinition::new(
                tool_schema("demo.remote"),
                McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
            )
            .expect("remote definition"),
        ])
    }

    fn accepts_remote_session_context(&self) -> bool {
        true
    }

    fn call_tool(
        &self,
        _name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.called.store(true, Ordering::SeqCst);
        Ok(json!({ "must_not_execute": true }))
    }
}

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
async fn d3_context_membership_filters_tool_list_and_call() {
    let agent_server = OrbitToolServer::new(Arc::new(CapabilityHost));
    let agent_context = agent_server.session_context();
    assert!(agent_context.has_capability(McpCapability::Agent));
    assert!(!agent_context.has_capability(McpCapability::Operator));
    let agent_names = agent_server
        .visible_tool_schemas()
        .expect("agent tool list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert!(agent_names.iter().any(|name| name == "demo.agent"));
    assert!(!agent_names.iter().any(|name| name == "demo.operator"));
    assert!(!agent_names.iter().any(|name| name == "demo.runner"));

    let called = agent_server
        .call_tool_request(CallToolRequestParams::new("demo_operator"))
        .await
        .expect("capability denial is a structured tool error");
    assert_eq!(called.is_error, Some(true));

    let mut operator_context = ToolSessionContext::trusted_local(None, None, None);
    operator_context.effective_capabilities = [McpCapability::Operator].into_iter().collect();
    assert!(operator_context.has_capability(McpCapability::Operator));
    assert!(!operator_context.has_capability(McpCapability::Agent));
    let operator_server =
        OrbitToolServer::new_with_context(Arc::new(CapabilityHost), operator_context);
    let operator_names = operator_server
        .visible_tool_schemas()
        .expect("operator tool list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert!(operator_names.iter().any(|name| name == "demo.operator"));
    assert!(!operator_names.iter().any(|name| name == "demo.agent"));
    assert!(!operator_names.iter().any(|name| name == "demo.runner"));
}

#[tokio::test]
async fn managed_empty_capability_set_is_never_upgraded_and_runner_is_non_hierarchical() {
    let empty =
        OrbitToolServer::new_with_context(Arc::new(CapabilityHost), ToolSessionContext::default());
    assert!(empty.visible_tool_schemas().expect("empty list").is_empty());
    let denied = empty
        .call_tool_request(CallToolRequestParams::new("demo_agent"))
        .await
        .expect("empty capability denial is structured");
    assert_eq!(denied.is_error, Some(true));

    let runner_context = ToolSessionContext {
        effective_capabilities: [McpCapability::Runner].into_iter().collect(),
        ..ToolSessionContext::default()
    };
    let runner = OrbitToolServer::new_with_context(Arc::new(CapabilityHost), runner_context);
    let names = runner
        .visible_tool_schemas()
        .expect("runner list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["demo.runner"]);
}

#[tokio::test]
async fn hub_tool_call_without_connector_metadata_fails_before_dispatch() {
    let host = Arc::new(RemoteMetadataHost {
        called: AtomicBool::new(false),
    });
    let mut trusted = ToolSessionContext::trusted_local(None, None, None);
    trusted.effective_capabilities = [McpCapability::Agent].into_iter().collect();
    let server_host: Arc<dyn crate::McpHost> = host.clone();
    let server = OrbitToolServer::new_with_context(server_host, trusted);

    let denied = server
        .call_tool_request(CallToolRequestParams::new("demo_remote"))
        .await
        .expect("missing metadata is a structured tool denial");

    assert_eq!(denied.is_error, Some(true));
    assert!(!host.called.load(Ordering::SeqCst));
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
