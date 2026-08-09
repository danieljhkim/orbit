use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use chrono::Utc;
use orbit_common::types::{McpCapability, Workspace, WorkspaceRegistry, WorkspaceStatus};
use orbit_remote::workspace_registry;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, ORIGIN};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::super::{build_app, serve_listener};
use crate::state::DashboardState;

const EXTERNAL_ORIGIN: &str = "https://mcp-client.example";
const MCP_ACCEPT: &str = "application/json, text/event-stream";
const WORKSPACE_ID: &str = "mcp-dashboard-test";

struct TestServer {
    _root: tempfile::TempDir,
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), orbit_core::OrbitError>>,
}

impl TestServer {
    async fn start(capability: Option<McpCapability>) -> Self {
        let root = tempfile::tempdir().expect("temp root");
        let global_root = root.path().join("global");
        std::fs::create_dir_all(&global_root).expect("global root");
        workspace_registry::save_registry_to(
            &WorkspaceRegistry {
                workspaces: vec![Workspace {
                    id: WORKSPACE_ID.to_string(),
                    name: "MCP dashboard test".to_string(),
                    owner_machine_id: None,
                    git_remote: None,
                    ship_mode: None,
                    base_branch: "agent-main".to_string(),
                    status: WorkspaceStatus::Active,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }],
                ..WorkspaceRegistry::default()
            },
            &workspace_registry::registry_path_for(&global_root),
        )
        .expect("workspace registry");
        let state = DashboardState::global(global_root, Vec::new(), None);
        let (app, mcp_control) = build_app(state, capability).expect("dashboard app");
        let listener =
            tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .await
                .expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(serve_listener(listener, app, mcp_control, async move {
            let _ = shutdown_rx.await;
        }));
        Self {
            _root: root,
            base_url: format!("http://{addr}"),
            shutdown: Some(shutdown),
            task,
        }
    }

    fn signal_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    async fn wait(mut self) {
        self.signal_shutdown();
        timeout(Duration::from_secs(2), self.task)
            .await
            .expect("server stops promptly")
            .expect("server task joins")
            .expect("server exits cleanly");
    }
}

async fn post_mcp(
    client: &reqwest::Client,
    base_url: &str,
    session_id: Option<&str>,
    message: Value,
) -> (HeaderMap, Value) {
    let mut request = client
        .post(format!("{base_url}/mcp"))
        .header(ACCEPT, MCP_ACCEPT)
        .header(CONTENT_TYPE, "application/json")
        .header(ORIGIN, EXTERNAL_ORIGIN)
        .json(&message);
    if let Some(session_id) = session_id {
        request = request.header("mcp-session-id", session_id);
    }
    let response = request.send().await.expect("MCP response");
    assert!(
        response.status().is_success(),
        "status: {}",
        response.status()
    );
    let headers = response.headers().clone();
    let body = response.text().await.expect("MCP response body");
    let message = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .find_map(|data| serde_json::from_str(data).ok())
        .unwrap_or_else(|| panic!("MCP SSE response contained no JSON message: {body}"));
    (headers, message)
}

async fn initialize(client: &reqwest::Client, base_url: &str) -> String {
    let (headers, response) = post_mcp(
        client,
        base_url,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "orbit-dashboard-test", "version": "1"},
                "_meta": {"orbit": {"workspace": WORKSPACE_ID}}
            }
        }),
    )
    .await;
    assert_eq!(response["id"], 1);
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("initialize session id")
        .to_string();

    let response = client
        .post(format!("{base_url}/mcp"))
        .header(ACCEPT, MCP_ACCEPT)
        .header(CONTENT_TYPE, "application/json")
        .header(ORIGIN, EXTERNAL_ORIGIN)
        .header("mcp-session-id", &session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .send()
        .await
        .expect("initialized notification");
    assert!(response.status().is_success());
    session_id
}

#[tokio::test]
async fn network_mcp_round_trip_keeps_origin_policy_and_default_capability_isolated() {
    let server = TestServer::start(None).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("HTTP client");
    let session_id = initialize(&client, &server.base_url).await;

    let (_, listed) = post_mcp(
        &client,
        &server.base_url,
        Some(&session_id),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let names = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"orbit_friction_tags"));
    assert!(
        !names.contains(&"orbit_workspace_list"),
        "no explicit capability must not expose operator-only tools"
    );

    let (_, called) = post_mcp(
        &client,
        &server.base_url,
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "orbit_friction_tags", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(called["id"], 3);
    assert_eq!(called["result"]["isError"], false);
    assert!(called["result"]["structuredContent"].is_object());

    let api_response = client
        .post(format!("{}/api/tasks", server.base_url))
        .header(ORIGIN, EXTERNAL_ORIGIN)
        .json(&json!({}))
        .send()
        .await
        .expect("API response");
    assert_eq!(api_response.status(), reqwest::StatusCode::FORBIDDEN);

    server.wait().await;
}

#[tokio::test]
async fn explicit_operator_capability_changes_the_advertised_surface() {
    let server = TestServer::start(Some(McpCapability::Operator)).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("HTTP client");
    let session_id = initialize(&client, &server.base_url).await;
    let (_, listed) = post_mcp(
        &client,
        &server.base_url,
        Some(&session_id),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let names = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"orbit_workspace_list"));
    server.wait().await;
}

#[tokio::test]
async fn mcp_stream_arrives_before_completion_and_shutdown_closes_the_session() {
    let mut server = TestServer::start(Some(McpCapability::Agent)).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("HTTP client");
    let session_id = initialize(&client, &server.base_url).await;

    let mut stream = client
        .get(format!("{}/mcp", server.base_url))
        .header(ACCEPT, "text/event-stream")
        .header(ORIGIN, EXTERNAL_ORIGIN)
        .header("mcp-session-id", &session_id)
        .send()
        .await
        .expect("open MCP stream");
    assert!(stream.status().is_success());
    let first = timeout(Duration::from_secs(1), stream.chunk())
        .await
        .expect("stream emits before completion")
        .expect("stream read")
        .expect("priming chunk");
    assert!(
        !first.is_empty(),
        "an open response must yield an incremental SSE frame"
    );

    server.signal_shutdown();
    timeout(Duration::from_secs(2), async {
        while stream.chunk().await.expect("stream read").is_some() {}
    })
    .await
    .expect("MCP stream closes during shutdown");
    timeout(Duration::from_secs(2), server.task)
        .await
        .expect("server stops with an MCP session open")
        .expect("server task joins")
        .expect("server exits cleanly");
}
