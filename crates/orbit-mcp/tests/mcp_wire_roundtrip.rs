//! Small wire-level proof for the generic MCP transport kernel.
#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError, ToolParam,
    ToolSchema, ToolSessionContext,
};
use orbit_mcp::{McpHost, OrbitToolServer};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Meta};
use serde_json::{Map, Value, json};
use tokio::io::duplex;

struct EchoHost {
    contexts: Mutex<Vec<ToolSessionContext>>,
}

impl McpHost for EchoHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let definition = McpToolDefinition::new(
            ToolSchema {
                name: "demo.echo".to_string(),
                description: "Echo one generic value.".to_string(),
                parameters: vec![ToolParam {
                    name: "value".to_string(),
                    description: "Value to echo.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                }],
                builtin: false,
            },
            McpToolPolicy::new(McpToolPlacement::LocalDerived, [McpCapability::Agent])
                .expect("fixture policy"),
        )
        .expect("fixture definition");
        Ok(vec![definition])
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        if name != "demo.echo" {
            return Err(OrbitError::not_found(
                orbit_common::types::NotFoundKind::Tool,
                name.to_string(),
            ));
        }
        self.contexts
            .lock()
            .expect("contexts")
            .push(context.clone());
        Ok(json!({
            "echo": input.get("value"),
            "workspace": context.workspace,
        }))
    }
}

#[tokio::test]
async fn generic_kernel_round_trips_initialize_list_call_and_error() {
    let host = Arc::new(EchoHost {
        contexts: Mutex::new(Vec::new()),
    });
    let server_host: Arc<dyn McpHost> = host.clone();
    let trusted = ToolSessionContext::trusted_local(None, None, None);
    let server = OrbitToolServer::new_with_context(server_host, trusted);

    let (client_io, server_io) = duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);
    let server_task = tokio::spawn(async move {
        let service = server
            .serve((server_read, server_write))
            .await
            .expect("serve MCP fixture");
        service.waiting().await.expect("wait for MCP fixture");
    });

    let mut client_info = ClientInfo::default();
    client_info.meta = Some(Meta(
        json!({ "orbit": { "workspace": "/tmp/generic-workspace" } })
            .as_object()
            .expect("initialize metadata")
            .clone(),
    ));
    let client = client_info
        .serve((client_read, client_write))
        .await
        .expect("connect MCP fixture");

    let initialized = client.peer_info().expect("initialize result");
    assert_eq!(initialized.server_info.name, "orbit-mcp");
    assert!(
        initialized
            .instructions
            .as_deref()
            .is_some_and(|instructions| instructions.contains("tools/list"))
    );

    let listed = client
        .peer()
        .list_tools(Default::default())
        .await
        .expect("tools/list");
    assert_eq!(listed.tools.len(), 1);
    let tool = &listed.tools[0];
    assert_eq!(tool.name.as_ref(), "demo_echo");
    assert_eq!(tool.input_schema["required"], json!(["value"]));
    assert!(
        tool.input_schema["properties"]["value"]
            .get("enum")
            .is_none()
    );

    let result = client
        .peer()
        .call_tool(call("demo_echo", json!({ "value": "hello" })))
        .await
        .expect("tools/call");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content,
        Some(json!({
            "echo": "hello",
            "workspace": "/tmp/generic-workspace",
        }))
    );

    let missing = client
        .peer()
        .call_tool(call("demo_missing", json!({})))
        .await
        .expect("unknown tools/call returns a structured error");
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(
        missing.structured_content.as_ref().unwrap()["code"],
        "tool_not_found"
    );

    let contexts = host.contexts.lock().expect("contexts");
    assert_eq!(contexts.len(), 1);
    assert_eq!(
        contexts[0].workspace.as_deref(),
        Some("/tmp/generic-workspace")
    );

    server_task.abort();
}

fn call(name: &str, args: Value) -> CallToolRequestParams {
    let arguments: Map<String, Value> = args
        .as_object()
        .expect("tool arguments are an object")
        .clone();
    CallToolRequestParams::new(name.to_string()).with_arguments(arguments)
}
