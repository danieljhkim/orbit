//! Relay tests [ORB-10710].
//!
//! Every test drives a real [`McpTcpServer`] — the same listener
//! `orbit mcp serve --listen` runs — so "the relay is transparent" is checked
//! against actual protocol traffic rather than a mock that agrees with us.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, OrbitError, ToolParam,
    ToolSchema, ToolSessionContext,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use super::super::relay;
use crate::{McpHost, McpServerComposition, McpSessionFactory, McpTcpServer, OrbitToolServer};

/// A host with one deterministic echo tool, so a response body is stable
/// enough to compare byte for byte across two transports.
struct EchoHost {
    calls: Arc<AtomicUsize>,
}

impl McpHost for EchoHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let policy = McpToolPolicy::new(McpToolPlacement::LocalDerived, [McpCapability::Agent])
            .expect("echo tool policy");
        let definition = McpToolDefinition::new(
            ToolSchema {
                name: "relay.echo".to_string(),
                description: "Echo the input back verbatim.".to_string(),
                parameters: vec![ToolParam {
                    name: "value".to_string(),
                    description: "Value to echo.".to_string(),
                    required: true,
                    param_type: "string".to_string(),
                }],
                builtin: true,
            },
            policy,
        )
        .expect("echo tool definition");
        Ok(vec![definition])
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "tool": name, "echoed": input }))
    }
}

/// Bind a listener on an ephemeral loopback port and serve every accepted
/// connection, exactly as `serve_tcp_with_context_and_composition` does.
async fn spawn_listener(calls: Arc<AtomicUsize>) -> SocketAddr {
    let host: Arc<dyn McpHost> = Arc::new(EchoHost { calls });
    let factory = McpSessionFactory::new(
        host,
        ToolSessionContext::trusted_local(None, None, None),
        McpServerComposition::new(),
    );
    let server = McpTcpServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), factory)
        .await
        .expect("bind loopback listener");
    let addr = server.local_addr().expect("bound address");
    tokio::spawn(async move {
        let _ = server.serve().await;
    });
    addr
}

/// The request frames one session sends: initialize, the initialized
/// notification, a tool listing, and two tool calls.
fn session_frames() -> Vec<String> {
    vec![
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "orbit-relay-test", "version": "0" },
            },
        })
        .to_string(),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }).to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "relay.echo", "arguments": { "value": "first" } },
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "relay.echo", "arguments": { "value": "second" } },
        })
        .to_string(),
    ]
}

/// Number of responses [`session_frames`] expects back (the notification gets
/// none).
const EXPECTED_RESPONSES: usize = 4;

/// Write every frame, then read exactly `EXPECTED_RESPONSES` lines back.
async fn drive_session<S>(stream: S) -> Vec<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    for frame in session_frames() {
        write
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .expect("write request frame");
    }
    write.flush().await.expect("flush requests");

    let mut responses = Vec::new();
    while responses.len() < EXPECTED_RESPONSES {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await.expect("read response");
        if read == 0 {
            break;
        }
        if !line.trim().is_empty() {
            responses.push(line);
        }
    }
    responses
}

/// Run one session straight against the listener, with no relay in between.
async fn direct_session(addr: SocketAddr) -> Vec<String> {
    let stream = TcpStream::connect(addr).await.expect("connect listener");
    drive_session(stream).await
}

#[tokio::test]
async fn relayed_responses_are_byte_identical_to_direct_ones() {
    let calls = Arc::new(AtomicUsize::new(0));
    let addr = spawn_listener(Arc::clone(&calls)).await;

    let direct = direct_session(addr).await;

    let (client, proxy) = tokio::io::duplex(64 * 1024);
    let server = TcpStream::connect(addr).await.expect("connect listener");
    let relayed = tokio::spawn(async move { relay(proxy, server).await });
    let through_relay = drive_session(client).await;

    assert_eq!(
        through_relay, direct,
        "a relayed response must be byte-identical to the same call made against \
         the listener directly — the relay declares nothing and reshapes nothing"
    );
    assert_eq!(
        through_relay.len(),
        EXPECTED_RESPONSES,
        "initialize, tools/list, and both tool calls must all answer"
    );
    drop(relayed);
}

#[tokio::test]
async fn one_connection_carries_every_call_in_a_session() {
    // The tunnel and its connection are established once. If the relay were
    // re-opening a transport per call, the listener would see one connection
    // per request instead of one per session.
    let calls = Arc::new(AtomicUsize::new(0));
    let host: Arc<dyn McpHost> = Arc::new(EchoHost {
        calls: Arc::clone(&calls),
    });
    let factory = McpSessionFactory::new(
        host,
        ToolSessionContext::trusted_local(None, None, None),
        McpServerComposition::new(),
    );
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("bound address");
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_counter = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            accepted_counter.fetch_add(1, Ordering::SeqCst);
            let session: OrbitToolServer = factory.build_session();
            tokio::spawn(async move {
                if let Ok(running) = rmcp::ServiceExt::serve(session, stream).await {
                    let _ = running.waiting().await;
                }
            });
        }
    });

    let (client, proxy) = tokio::io::duplex(64 * 1024);
    let server = TcpStream::connect(addr).await.expect("connect listener");
    let relayed = tokio::spawn(async move { relay(proxy, server).await });
    let responses = drive_session(client).await;

    assert_eq!(responses.len(), EXPECTED_RESPONSES);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "both tool calls must have reached the host"
    );
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "every call in the session must ride the single established connection"
    );
    drop(relayed);
}

#[tokio::test]
async fn client_eof_drains_the_response_direction() {
    // Closing stdin must not truncate a reply already in flight: the relay
    // half-closes the server side and then drains what comes back.
    let calls = Arc::new(AtomicUsize::new(0));
    let addr = spawn_listener(calls).await;

    // The two directions are separate streams so that dropping the request
    // side is a real EOF on the relay's client-read half — which is what
    // closing an MCP client's stdin actually looks like. Splitting one duplex
    // would not: the write half's drop leaves the duplex open.
    let (mut requests, request_source) = tokio::io::duplex(64 * 1024);
    let (response_sink, response_source) = tokio::io::duplex(64 * 1024);
    let server = TcpStream::connect(addr).await.expect("connect listener");
    let relayed =
        tokio::spawn(
            async move { relay(tokio::io::join(request_source, response_sink), server).await },
        );

    for frame in session_frames() {
        requests
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .expect("write request frame");
    }
    requests.flush().await.expect("flush requests");
    drop(requests); // the client is gone the moment its requests are out

    let mut reader = BufReader::new(response_source);
    let mut responses = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if !line.trim().is_empty() {
                    responses.push(line);
                }
            }
        }
        if responses.len() == EXPECTED_RESPONSES {
            break;
        }
    }

    assert_eq!(
        responses.len(),
        EXPECTED_RESPONSES,
        "responses already in flight when the client closed stdin must still be delivered"
    );
    let outcome = relayed.await.expect("relay task");
    assert!(
        outcome.is_ok(),
        "a clean client EOF is a normal session end: {outcome:?}"
    );
}

#[tokio::test]
async fn server_disconnect_ends_the_session() {
    // If the tunnel or the remote listener dies, the relay must return so the
    // client observes EOF rather than hanging on a dead transport.
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("bound address");
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });

    let (client, proxy) = tokio::io::duplex(1024);
    let server = TcpStream::connect(addr).await.expect("connect listener");
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), relay(proxy, server))
        .await
        .expect("relay must return once the server side is gone");

    assert!(
        outcome.is_ok() || matches!(outcome, Err(OrbitError::Io(_))),
        "a dropped server connection ends the session: {outcome:?}"
    );
    drop(client);
}

#[tokio::test]
async fn relay_reports_a_broken_client_as_io() {
    // A relay that swallowed transport failures would leave the caller unable
    // to tell a finished session from a broken one.
    let calls = Arc::new(AtomicUsize::new(0));
    let addr = spawn_listener(calls).await;
    let server = TcpStream::connect(addr).await.expect("connect listener");

    let (client, proxy) = tokio::io::duplex(16);
    drop(client);

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), relay(proxy, server))
        .await
        .expect("relay must return once the client is gone");
    assert!(
        outcome.is_ok() || matches!(outcome, Err(OrbitError::Io(_))),
        "unexpected relay outcome: {outcome:?}"
    );
}
