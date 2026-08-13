use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError, ToolSchema,
    ToolSessionContext,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::super::OrbitToolServer;
use super::super::name_map::sanitize_tool_name;
use super::super::test_support::{
    EchoArrayHost, StubHost, request_with_args, test_mcp_definitions, tool_schema,
};
use crate::{McpToolExtension, McpToolExtensionRegistration};

struct MissingPolicyHost;

struct CapabilityHost;

struct ExtensionPolicyHost {
    schemas: Vec<ToolSchema>,
    host_calls: AtomicUsize,
    in_process_calls: AtomicUsize,
}

struct EchoExtension {
    calls: AtomicUsize,
}

impl McpToolExtension for EchoExtension {
    fn definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions(vec![tool_schema("demo.extension")])
    }

    fn recognizes(&self, name: &str) -> bool {
        name == "demo.extension"
    }

    fn call(
        &self,
        name: &str,
        input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "tool": name, "input": input }))
    }

    fn input_schema(
        &self,
        _definition: &McpToolDefinition,
    ) -> Result<crate::McpInputSchema, OrbitError> {
        Ok(json!({
            "type": "object",
            "properties": {
                "value": { "type": "integer", "minimum": 1 }
            },
            "required": ["value"],
            "additionalProperties": false
        })
        .as_object()
        .expect("schema object")
        .clone())
    }
}

impl crate::McpHost for ExtensionPolicyHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions(self.schemas.clone())
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.host_calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "host_tool": name }))
    }

    fn call_in_process_tool(
        &self,
        _name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        self.in_process_calls.fetch_add(1, Ordering::SeqCst);
        dispatch(input, session_context)
    }
}

impl crate::McpHost for CapabilityHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        [
            ("demo.agent", vec![McpCapability::Agent]),
            ("demo.operator", vec![McpCapability::Operator]),
            (
                "demo.both",
                vec![McpCapability::Agent, McpCapability::Operator],
            ),
        ]
        .into_iter()
        .map(|(name, capabilities)| {
            let policy = McpToolPolicy::new(McpToolPlacement::Owner, capabilities)
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

#[tokio::test]
async fn explicit_extension_is_advertised_and_crosses_host_policy_seam() {
    let host = Arc::new(ExtensionPolicyHost {
        schemas: vec![tool_schema("demo.host")],
        host_calls: AtomicUsize::new(0),
        in_process_calls: AtomicUsize::new(0),
    });
    let extension = Arc::new(EchoExtension {
        calls: AtomicUsize::new(0),
    });
    let extension_handler: Arc<dyn McpToolExtension> = extension.clone();
    let server = OrbitToolServer::new_with_extensions(
        host.clone(),
        vec![McpToolExtensionRegistration::advertised(extension_handler)],
    );

    let names = server
        .combined_tool_schemas()
        .expect("combined extension definitions")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name == "demo.host"));
    assert!(names.iter().any(|name| name == "demo.extension"));

    let result = server
        .call_tool_request(request_with_args("demo.extension", json!({ "value": 7 })))
        .await
        .expect("extension call succeeds");
    assert_eq!(
        result.structured_content.expect("structured response"),
        json!({ "tool": "demo.extension", "input": { "value": 7 } })
    );
    assert_eq!(extension.calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.in_process_calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.host_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn extension_owns_its_complete_advertised_input_schema() {
    let host = Arc::new(ExtensionPolicyHost {
        schemas: Vec::new(),
        host_calls: AtomicUsize::new(0),
        in_process_calls: AtomicUsize::new(0),
    });
    let extension: Arc<dyn McpToolExtension> = Arc::new(EchoExtension {
        calls: AtomicUsize::new(0),
    });
    let server = OrbitToolServer::new_with_extensions(
        host,
        vec![McpToolExtensionRegistration::advertised(extension)],
    );
    let definition = server
        .combined_tool_definitions()
        .expect("extension definitions")
        .into_iter()
        .find(|definition| definition.schema.name == "demo.extension")
        .expect("extension definition");

    let schema = server
        .input_schema_for(&definition)
        .expect("extension input schema");

    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["value"]["minimum"], 1);
    assert_eq!(schema["required"], json!(["value"]));
}

#[tokio::test]
async fn recognition_only_extension_stays_hidden_and_owns_guessed_calls() {
    let host = Arc::new(ExtensionPolicyHost {
        schemas: vec![tool_schema("demo.extension")],
        host_calls: AtomicUsize::new(0),
        in_process_calls: AtomicUsize::new(0),
    });
    let extension = Arc::new(EchoExtension {
        calls: AtomicUsize::new(0),
    });
    let extension_handler: Arc<dyn McpToolExtension> = extension.clone();
    let server = OrbitToolServer::new_with_extensions(
        host.clone(),
        vec![McpToolExtensionRegistration::recognition_only(
            extension_handler,
        )],
    );

    assert!(
        server
            .combined_tool_schemas()
            .expect("hidden extension composition")
            .is_empty()
    );
    let result = server
        .call_tool_request(CallToolRequestParams::new("demo.extension"))
        .await
        .expect("guessed canonical call reaches the extension");
    assert!(!result.is_error.unwrap_or(false));
    assert_eq!(extension.calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.in_process_calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.host_calls.load(Ordering::SeqCst), 0);
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
    assert!(agent_names.iter().any(|name| name == "demo.both"));

    let called = agent_server
        .call_tool_request(CallToolRequestParams::new("demo_operator"))
        .await
        .expect("capability denial is a structured tool error");
    assert_eq!(called.is_error, Some(true));
    let structured = called
        .structured_content
        .expect("capability denial has structured content");
    assert_eq!(structured["code"], "capability_denied");

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
    assert!(operator_names.iter().any(|name| name == "demo.both"));
}

#[tokio::test]
async fn managed_empty_capability_set_is_never_upgraded_and_capabilities_are_non_hierarchical() {
    let empty =
        OrbitToolServer::new_with_context(Arc::new(CapabilityHost), ToolSessionContext::default());
    assert!(empty.visible_tool_schemas().expect("empty list").is_empty());
    let denied = empty
        .call_tool_request(CallToolRequestParams::new("demo_agent"))
        .await
        .expect("empty capability denial is structured");
    assert_eq!(denied.is_error, Some(true));

    // `operator` never reaches an agent-only tool, so the shared `demo.both`
    // entry is the only overlap between the two flat capability sets.
    let operator_context = ToolSessionContext {
        effective_capabilities: [McpCapability::Operator].into_iter().collect(),
        ..ToolSessionContext::default()
    };
    let operator = OrbitToolServer::new_with_context(Arc::new(CapabilityHost), operator_context);
    let names = operator
        .visible_tool_schemas()
        .expect("operator list")
        .into_iter()
        .map(|schema| schema.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["demo.operator", "demo.both"]);
}

#[tokio::test]
async fn call_tool_wraps_affected_array_results_for_strict_mcp_clients() {
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
