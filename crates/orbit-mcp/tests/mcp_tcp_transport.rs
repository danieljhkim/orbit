//! Wire-level proof that the TCP transport isolates sessions and advertises
//! the same surface stdio does.
#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError, ToolParam,
    ToolSchema, ToolSessionContext,
};
use orbit_mcp::{
    McpHost, McpServerComposition, McpServerMetadata, McpSessionFactory, McpTcpServer,
    OrbitToolServer,
};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo, Meta, Tool};
use serde_json::{Map, Value, json};
use tokio::io::duplex;
use tokio::net::TcpStream;

const CONTRACT_INSTRUCTIONS: &str = "Orbit hub contract v1. Call tools/list before dispatching.";

/// A host with one agent-visible tool and one operator-only tool, so a session
/// holding only the agent capability exercises both the hidden-from-listing and
/// the refused-on-call paths.
struct CapabilityHost {
    contexts: Mutex<Vec<ToolSessionContext>>,
}

impl CapabilityHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            contexts: Mutex::new(Vec::new()),
        })
    }

    fn observed_workspaces(&self) -> Vec<Option<String>> {
        self.contexts
            .lock()
            .expect("observed contexts")
            .iter()
            .map(|context| context.workspace.clone())
            .collect()
    }
}

fn definition(name: &str, capability: McpCapability) -> McpToolDefinition {
    McpToolDefinition::new(
        ToolSchema {
            name: name.to_string(),
            description: "Fixture tool.".to_string(),
            parameters: vec![ToolParam {
                name: "value".to_string(),
                description: "Value to echo.".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: false,
        },
        McpToolPolicy::new(McpToolPlacement::LocalDerived, [capability]).expect("fixture policy"),
    )
    .expect("fixture definition")
}

impl McpHost for CapabilityHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(vec![
            definition("demo.echo", McpCapability::Agent),
            definition("demo.operate", McpCapability::Operator),
        ])
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        if !matches!(name, "demo.echo" | "demo.operate") {
            return Err(OrbitError::not_found(
                orbit_common::types::NotFoundKind::Tool,
                name.to_string(),
            ));
        }
        self.contexts
            .lock()
            .expect("observed contexts")
            .push(context.clone());
        Ok(json!({
            "tool": name,
            "echo": input.get("value"),
            "workspace": context.workspace,
            "capabilities": context
                .effective_capabilities
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        }))
    }
}

fn composition() -> McpServerComposition {
    McpServerComposition::new()
        .with_metadata(McpServerMetadata::default().with_instructions(CONTRACT_INSTRUCTIONS))
}

fn trusted_context(capability: McpCapability) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(None, None, None);
    context.effective_capabilities = BTreeSet::from([capability]);
    context
}

fn factory(host: Arc<CapabilityHost>, capability: McpCapability) -> McpSessionFactory {
    McpSessionFactory::new(host, trusted_context(capability), composition())
}

/// Bind a listener on an ephemeral loopback port and start accepting.
async fn start_endpoint(
    host: Arc<CapabilityHost>,
    capability: McpCapability,
) -> (SocketAddr, tokio::task::JoinHandle<Result<(), OrbitError>>) {
    let server = McpTcpServer::bind(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        factory(host, capability),
    )
    .await
    .expect("bind loopback endpoint");
    let addr = server.local_addr().expect("bound address");
    (addr, tokio::spawn(server.serve()))
}

fn client_info(meta: Option<Value>) -> ClientInfo {
    let mut info = ClientInfo::default();
    info.meta = meta.map(|meta| {
        Meta(
            meta.as_object()
                .expect("initialize metadata is an object")
                .clone(),
        )
    });
    info
}

fn workspace_meta(workspace: &str) -> Value {
    json!({ "orbit": { "workspace": workspace } })
}

fn call(name: &str, args: Value) -> CallToolRequestParams {
    let arguments: Map<String, Value> = args
        .as_object()
        .expect("tool arguments are an object")
        .clone();
    CallToolRequestParams::new(name.to_string()).with_arguments(arguments)
}

fn advertised(tools: &[Tool]) -> Vec<(String, Value)> {
    let mut surface = tools
        .iter()
        .map(|tool| {
            (
                tool.name.to_string(),
                Value::Object((*tool.input_schema).clone()),
            )
        })
        .collect::<Vec<_>>();
    surface.sort_by(|left, right| left.0.cmp(&right.0));
    surface
}

fn error_payload(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "expected a refusal");
    result
        .structured_content
        .clone()
        .expect("structured error payload")
}

#[tokio::test]
async fn concurrent_tcp_clients_never_observe_another_session_workspace() {
    let host = CapabilityHost::new();
    let (addr, endpoint) = start_endpoint(Arc::clone(&host), McpCapability::Agent).await;

    // Both clients complete `initialize` before either calls a tool. On a
    // server whose session state is shared across connections, the second
    // initialize overwrites the first client's workspace selection and the
    // first client's call then succeeds against the wrong workspace — wrong
    // data returned as a success, which is what this ordering catches.
    let alpha = client_info(Some(workspace_meta("/tmp/alpha")))
        .serve(TcpStream::connect(addr).await.expect("connect alpha"))
        .await
        .expect("initialize alpha");
    let beta = client_info(Some(workspace_meta("/tmp/beta")))
        .serve(TcpStream::connect(addr).await.expect("connect beta"))
        .await
        .expect("initialize beta");

    let alpha_result = alpha
        .peer()
        .call_tool(call("demo_echo", json!({ "value": "a" })))
        .await
        .expect("alpha tools/call");
    let beta_result = beta
        .peer()
        .call_tool(call("demo_echo", json!({ "value": "b" })))
        .await
        .expect("beta tools/call");

    assert_eq!(
        alpha_result.structured_content.as_ref().unwrap()["workspace"],
        json!("/tmp/alpha")
    );
    assert_eq!(
        beta_result.structured_content.as_ref().unwrap()["workspace"],
        json!("/tmp/beta")
    );

    let mut observed = host.observed_workspaces();
    observed.sort();
    assert_eq!(
        observed,
        vec![
            Some("/tmp/alpha".to_string()),
            Some("/tmp/beta".to_string())
        ]
    );

    endpoint.abort();
}

#[tokio::test]
async fn tcp_advertises_and_refuses_exactly_as_stdio_does() {
    let host = CapabilityHost::new();
    let (addr, endpoint) = start_endpoint(Arc::clone(&host), McpCapability::Agent).await;

    // The stdio entry points construct this server and hand it the process's
    // standard streams; only the byte source differs from the socket below.
    let stdio_server = OrbitToolServer::new_with_context_and_composition(
        Arc::clone(&host) as Arc<dyn McpHost>,
        trusted_context(McpCapability::Agent),
        composition(),
    );
    let (client_io, server_io) = duplex(64 * 1024);
    let stdio_task = tokio::spawn(async move {
        let service = stdio_server.serve(server_io).await.expect("serve stdio");
        service.waiting().await.expect("stdio session");
    });

    let over_tcp = client_info(Some(workspace_meta("/tmp/parity")))
        .serve(TcpStream::connect(addr).await.expect("connect tcp"))
        .await
        .expect("initialize over tcp");
    let over_stdio = client_info(Some(workspace_meta("/tmp/parity")))
        .serve(client_io)
        .await
        .expect("initialize over stdio");

    let tcp_tools = over_tcp
        .peer()
        .list_tools(Default::default())
        .await
        .expect("tcp tools/list");
    let stdio_tools = over_stdio
        .peer()
        .list_tools(Default::default())
        .await
        .expect("stdio tools/list");

    assert_eq!(advertised(&tcp_tools.tools), advertised(&stdio_tools.tools));
    assert_eq!(
        advertised(&tcp_tools.tools)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        vec!["demo_echo".to_string()],
        "the operator-only tool is filtered out of an agent session's listing"
    );

    // Hidden by capability: reached by name, refused identically on both.
    let tcp_denied = over_tcp
        .peer()
        .call_tool(call("demo_operate", json!({ "value": "x" })))
        .await
        .expect("tcp refusal is a structured result");
    let stdio_denied = over_stdio
        .peer()
        .call_tool(call("demo_operate", json!({ "value": "x" })))
        .await
        .expect("stdio refusal is a structured result");
    assert_eq!(error_payload(&tcp_denied), error_payload(&stdio_denied));
    assert_eq!(
        error_payload(&tcp_denied)["code"],
        json!("capability_denied")
    );

    // Not a tool at all: same classification on both.
    let tcp_missing = over_tcp
        .peer()
        .call_tool(call("demo_absent", json!({})))
        .await
        .expect("tcp unknown-name result");
    let stdio_missing = over_stdio
        .peer()
        .call_tool(call("demo_absent", json!({})))
        .await
        .expect("stdio unknown-name result");
    assert_eq!(error_payload(&tcp_missing), error_payload(&stdio_missing));
    assert_eq!(error_payload(&tcp_missing)["code"], json!("tool_not_found"));

    assert_eq!(
        over_tcp
            .peer_info()
            .expect("tcp initialize result")
            .instructions
            .as_deref(),
        Some(CONTRACT_INSTRUCTIONS),
        "composition instructions survive the network initialize"
    );
    assert_eq!(
        over_tcp
            .peer_info()
            .expect("tcp initialize result")
            .instructions,
        over_stdio
            .peer_info()
            .expect("stdio initialize result")
            .instructions
    );

    endpoint.abort();
    stdio_task.abort();
}

#[tokio::test]
async fn client_announced_metadata_cannot_widen_session_capability() {
    let host = CapabilityHost::new();
    let (addr, endpoint) = start_endpoint(Arc::clone(&host), McpCapability::Agent).await;

    let escalating = client_info(Some(json!({
        "orbit": {
            "workspace": "/tmp/escalate",
            "capabilities": ["operator", "runner"],
            "effective_capabilities": ["operator"],
            "caller_machine_id": "attacker-machine",
            "transport": "hub",
        },
        "orbit.capabilities": ["operator"],
    })))
    .serve(TcpStream::connect(addr).await.expect("connect"))
    .await
    .expect("initialize");

    let listed = escalating
        .peer()
        .list_tools(Default::default())
        .await
        .expect("tools/list");
    assert_eq!(
        listed
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>(),
        vec!["demo_echo".to_string()]
    );

    let denied = escalating
        .peer()
        .call_tool(call("demo_operate", json!({ "value": "x" })))
        .await
        .expect("structured refusal");
    assert_eq!(error_payload(&denied)["code"], json!("capability_denied"));

    let permitted = escalating
        .peer()
        .call_tool(call("demo_echo", json!({ "value": "x" })))
        .await
        .expect("tools/call");
    let observed = permitted.structured_content.expect("structured result");
    assert_eq!(observed["capabilities"], json!(["agent"]));
    assert_eq!(
        observed["workspace"],
        json!("/tmp/escalate"),
        "the announced workspace selector is the only field a client controls"
    );

    let recorded = host.contexts.lock().expect("observed contexts");
    let context = recorded.last().expect("one recorded call");
    assert_eq!(context.caller_machine_id, None);
    assert_eq!(
        context.transport,
        ToolSessionContext::trusted_local(None, None, None).transport
    );

    drop(recorded);
    endpoint.abort();
}
