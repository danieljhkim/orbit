use std::sync::{Arc, Mutex};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError,
    ToolSessionContext,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::super::OrbitToolServer;
use super::super::name_map::sanitize_tool_name;
use super::super::test_support::{EchoArrayHost, StubHost, tool_schema};

struct PolicyTaggedHost {
    calls: Mutex<Vec<(String, ToolSessionContext)>>,
}

impl PolicyTaggedHost {
    fn definition(
        name: &str,
        capabilities: impl IntoIterator<Item = McpCapability>,
    ) -> Result<McpToolDefinition, OrbitError> {
        let policy = McpToolPolicy::new(McpToolPlacement::Owner, capabilities)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        McpToolDefinition::new(tool_schema(name), policy)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))
    }
}

impl crate::McpHost for PolicyTaggedHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(vec![
            Self::definition("demo.agent", [McpCapability::Agent])?,
            Self::definition("demo.operator", [McpCapability::Operator])?,
            Self::definition("demo.both", [McpCapability::Agent, McpCapability::Operator])?,
        ])
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((name.to_string(), context));
        Ok(json!({ "tool": name }))
    }
}

struct InvalidNameHost;

impl crate::McpHost for InvalidNameHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        PolicyTaggedHost::definition("", [McpCapability::Agent]).map(|definition| vec![definition])
    }

    fn call_tool(
        &self,
        _name: &str,
        _input: Value,
        _context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(Value::Null)
    }
}

#[tokio::test]
async fn policy_tags_do_not_filter_the_list_or_call_surface() {
    let host = Arc::new(PolicyTaggedHost {
        calls: Mutex::new(Vec::new()),
    });
    let server = OrbitToolServer::new_with_context(host.clone(), ToolSessionContext::default());

    let names = server
        .tool_schemas()
        .expect("complete tool list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["demo.agent", "demo.operator", "demo.both"]);

    for name in ["demo_agent", "demo_operator", "demo_both"] {
        let result = server
            .call_tool_request(CallToolRequestParams::new(name))
            .await
            .expect("every listed tool reaches the host");
        assert_eq!(result.is_error, Some(false));
    }

    let calls = host.calls.lock().expect("calls lock");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].0, "demo.agent");
    assert_eq!(calls[1].0, "demo.operator");
    assert_eq!(calls[2].0, "demo.both");
}

#[tokio::test]
async fn every_tool_call_gets_one_fresh_trace_without_rewriting_legacy_call_id() {
    let host = Arc::new(PolicyTaggedHost {
        calls: Mutex::new(Vec::new()),
    });
    let trusted = ToolSessionContext {
        mcp_call_id: Some("legacy-call".to_string()),
        ..ToolSessionContext::default()
    };
    let server = OrbitToolServer::new_with_context(host.clone(), trusted);

    for _ in 0..2 {
        server
            .call_tool_request(CallToolRequestParams::new("demo_agent"))
            .await
            .expect("call succeeds");
    }

    let calls = host.calls.lock().expect("calls lock");
    let first = &calls[0].1;
    let second = &calls[1].1;
    assert!(
        first
            .trace_id
            .as_deref()
            .is_some_and(|id| id.starts_with("trace-"))
    );
    assert!(
        second
            .trace_id
            .as_deref()
            .is_some_and(|id| id.starts_with("trace-"))
    );
    assert_ne!(first.trace_id, second.trace_id);
    assert_eq!(first.mcp_call_id.as_deref(), Some("legacy-call"));
    assert_eq!(second.mcp_call_id.as_deref(), Some("legacy-call"));
}

#[test]
fn invalid_canonical_names_fail_the_surface_before_dispatch() {
    let server = OrbitToolServer::new(Arc::new(InvalidNameHost));
    let error = server
        .tool_schemas()
        .expect_err("empty canonical name is invalid");
    assert!(error.to_string().contains("must not be empty"));
}

#[tokio::test]
async fn array_results_are_object_shaped_for_strict_mcp_clients() {
    let affected_tools = ["orbit.task.list", "orbit.friction.list"];
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
            .expect("MCP call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");
        assert_eq!(
            structured.get("items"),
            Some(&json!([{ "tool": canonical_name }]))
        );
        let wire = serde_json::to_value(&result).expect("serialize result");
        assert!(
            wire.get("structuredContent").is_some_and(Value::is_object),
            "{canonical_name} must satisfy object-only clients"
        );
    }
}

#[test]
fn canonical_name_translates_advertised_back_to_dotted() {
    let server = OrbitToolServer::new(Arc::new(StubHost {
        schemas: vec![tool_schema("orbit.task.add")],
    }));
    assert_eq!(
        server.canonical_name("orbit_task_add").unwrap(),
        "orbit.task.add"
    );
    assert_eq!(
        server.canonical_name("orbit_task_add").unwrap(),
        "orbit.task.add"
    );
}

#[test]
fn canonical_name_passes_unknown_and_legacy_dotted_names_to_the_host() {
    let server = OrbitToolServer::new(Arc::new(StubHost {
        schemas: vec![tool_schema("orbit.task.add")],
    }));
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
    let server = OrbitToolServer::new(Arc::new(StubHost {
        schemas: vec![tool_schema("foo.bar"), tool_schema("foo_bar")],
    }));
    let error = server
        .canonical_name("foo_bar")
        .expect_err("dispatch must reject ambiguous advertised names");
    assert!(
        error
            .message
            .contains("invalid canonical MCP tool definitions")
    );
}
