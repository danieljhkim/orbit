//! Wire-level integration tests for the orbit-mcp server adapter.
//!
//! Each test boots the real [`OrbitToolServer`] on rmcp's transport runtime
//! over an in-memory duplex pipe and speaks raw newline-delimited JSON-RPC
//! bytes to it — the exact framing `orbit mcp serve` uses over stdio — so the
//! full serialize → handshake → dispatch → execute → serialize path is
//! exercised, not handler methods in isolation.
//!
//! The host is a temp-dir-backed fixture that persists task records as JSON
//! files, which keeps the tests hermetic (no `~/.orbit`, no network) while
//! still round-tripping every byte through the adapter. The full
//! runtime-backed surface (`RuntimeMcpHost`) is covered end-to-end by
//! `crates/orbit-cli/tests/mcp_roundtrip.rs`, which spawns the actual
//! `orbit mcp serve` binary.
#![allow(missing_docs)]
// ORB-00013: tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use orbit_common::types::{NotFoundKind, OrbitError, ToolParam, ToolSchema, ToolSessionContext};
use orbit_mcp::{McpHost, OrbitToolServer};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::time::timeout;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Path of the checked-in `tools/list` snapshot, relative to the crate root.
const SNAPSHOT_RELATIVE_PATH: &str = "tests/snapshots/wire_tools_list.json";

// ---------------------------------------------------------------------------
// Fixture host: persists task records as JSON files under a temp workspace.
// ---------------------------------------------------------------------------

struct FileStoreHost {
    root: PathBuf,
    calls: Mutex<Vec<(String, Value, Option<String>)>>,
}

impl FileStoreHost {
    fn new(root: PathBuf) -> Self {
        std::fs::create_dir_all(root.join("tasks")).expect("create task store dir");
        Self {
            root,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recorded_calls(&self) -> Vec<(String, Value, Option<String>)> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn task_path(&self, id: &str) -> PathBuf {
        self.root.join("tasks").join(format!("{id}.json"))
    }

    fn list_tasks(&self) -> Vec<Value> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(self.root.join("tasks"))
            .expect("read task store dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        entries.sort();
        entries
            .into_iter()
            .map(|path| {
                serde_json::from_str(&std::fs::read_to_string(path).expect("read task file"))
                    .expect("parse task file")
            })
            .collect()
    }

    fn task_add(
        &self,
        input: &Value,
        session_workspace: Option<&str>,
    ) -> Result<Value, OrbitError> {
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .ok_or_else(|| OrbitError::InvalidInput("missing `title`".to_string()))?;
        let id = format!("T-{:04}", self.list_tasks().len() + 1);
        let record = json!({
            "id": id,
            "title": title,
            "type": input.get("type").cloned().unwrap_or_else(|| json!("chore")),
            "tags": input.get("tags").cloned().unwrap_or_else(|| json!([])),
            "session_workspace": session_workspace,
        });
        std::fs::write(
            self.task_path(&id),
            serde_json::to_string_pretty(&record).expect("serialize task"),
        )
        .map_err(OrbitError::from)?;
        Ok(record)
    }

    fn task_show(&self, input: &Value) -> Result<Value, OrbitError> {
        let id = input
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| OrbitError::InvalidInput("missing `id`".to_string()))?;
        let path = self.task_path(id);
        if !path.exists() {
            return Err(OrbitError::not_found(NotFoundKind::Task, id.to_string()));
        }
        Ok(
            serde_json::from_str(&std::fs::read_to_string(path).map_err(OrbitError::from)?)
                .expect("parse task file"),
        )
    }
}

impl McpHost for FileStoreHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        fn param(name: &str, description: &str, param_type: &str, required: bool) -> ToolParam {
            ToolParam {
                name: name.to_string(),
                description: description.to_string(),
                param_type: param_type.to_string(),
                required,
            }
        }
        fn schema(name: &str, description: &str, parameters: Vec<ToolParam>) -> ToolSchema {
            ToolSchema {
                name: name.to_string(),
                description: description.to_string(),
                parameters,
                builtin: true,
            }
        }
        vec![
            schema(
                "orbit.task.add",
                "Create a task record in the fixture store.",
                vec![
                    param("title", "Task title", "string", true),
                    param("type", "Optional task type", "string", false),
                    param("tags", "Optional tags", "string_list", false),
                ],
            ),
            schema(
                "orbit.task.show",
                "Show one task record from the fixture store.",
                vec![param("id", "Task id", "string", true)],
            ),
            schema(
                "orbit.task.list",
                "List every task record in the fixture store.",
                Vec::new(),
            ),
        ]
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.calls.lock().expect("calls lock").push((
            name.to_string(),
            input.clone(),
            session_context.workspace.clone(),
        ));
        match name {
            "orbit.task.add" => self.task_add(&input, session_context.workspace.as_deref()),
            "orbit.task.show" => self.task_show(&input),
            "orbit.task.list" => Ok(Value::Array(self.list_tasks())),
            other => Err(OrbitError::not_found(NotFoundKind::Tool, other.to_string())),
        }
    }

    fn call_in_process_tool(
        &self,
        _name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        dispatch(input, session_context)
    }
}

// ---------------------------------------------------------------------------
// Wire client: raw newline-delimited JSON-RPC over an in-memory duplex pipe.
// ---------------------------------------------------------------------------

struct WireClient {
    reader: BufReader<ReadHalf<tokio::io::DuplexStream>>,
    writer: WriteHalf<tokio::io::DuplexStream>,
    next_id: i64,
}

impl WireClient {
    /// Boot `server` on rmcp's runtime over an in-memory pipe and return a
    /// client speaking raw JSON-RPC bytes to it. The server task is detached;
    /// it shuts down when the client half drops.
    fn start(server: OrbitToolServer) -> Self {
        let (server_io, client_io) = tokio::io::duplex(256 * 1024);
        tokio::spawn(async move {
            // `serve` drives the initialize handshake; `waiting` runs the
            // request loop until the peer disconnects.
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let (read_half, write_half) = tokio::io::split(client_io);
        Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            next_id: 0,
        }
    }

    async fn send_line(&mut self, message: &Value) {
        let mut line = serde_json::to_string(message).expect("serialize message");
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .expect("write JSON-RPC line");
    }

    async fn notify(&mut self, method: &str) {
        self.send_line(&json!({ "jsonrpc": "2.0", "method": method }))
            .await;
    }

    /// Send a request and await the response with a matching id, skipping any
    /// interleaved notifications.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send_line(&message).await;
        timeout(REQUEST_TIMEOUT, self.read_response(id))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for response to `{method}` (id {id})"))
    }

    async fn read_response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            let read = self
                .reader
                .read_line(&mut line)
                .await
                .expect("read JSON-RPC line");
            assert!(read > 0, "server closed the transport before responding");
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(line.trim())
                .unwrap_or_else(|err| panic!("server emitted invalid JSON ({err}): {line}"));
            if message.get("id").and_then(Value::as_i64) == Some(id) {
                return message;
            }
        }
    }

    async fn initialize(&mut self, meta: Option<Value>) -> Value {
        let mut params = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "orbit-mcp-wire-test", "version": "0" },
        });
        if let Some(meta) = meta {
            params["_meta"] = meta;
        }
        let response = self.request("initialize", params).await;
        self.notify("notifications/initialized").await;
        response
    }

    /// `tools/call` helper returning the whole `result` object.
    async fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let response = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("tools/call `{name}` returned no result: {response}"))
    }
}

fn structured(result: &Value) -> &Value {
    result
        .get("structuredContent")
        .unwrap_or_else(|| panic!("missing structuredContent in {result}"))
}

async fn start_fixture() -> (WireClient, std::sync::Arc<FileStoreHost>, TempDir) {
    let workspace = TempDir::new().expect("temp workspace");
    let host = std::sync::Arc::new(FileStoreHost::new(workspace.path().to_path_buf()));
    let client = WireClient::start(OrbitToolServer::new(host.clone()));
    (client, host, workspace)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn initialize_handshake_reports_protocol_version_server_info_and_capabilities() {
    let (mut client, _host, _workspace) = start_fixture().await;

    let response = client.initialize(None).await;
    let result = response.get("result").expect("initialize result");

    assert_eq!(
        result["protocolVersion"], "2025-06-18",
        "server must negotiate down to the client's protocol version"
    );
    assert_eq!(result["serverInfo"]["name"], "orbit-mcp");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        result["capabilities"]["tools"].is_object(),
        "tools capability must be advertised: {result}"
    );
    assert!(
        result["instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.contains("tools/list")),
        "instructions should point clients at tools/list: {result}"
    );
}

#[tokio::test]
async fn tools_list_matches_wire_snapshot() {
    let (mut client, _host, _workspace) = start_fixture().await;
    client.initialize(None).await;

    let response = client.request("tools/list", Value::Null).await;
    let tools = response["result"]["tools"].clone();

    // Advertised names are dot-sanitized and the in-process orbit-graph
    // surface is merged in alongside the host tools.
    let names: Vec<&str> = tools
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert!(names.contains(&"orbit_task_add"), "names: {names:?}");
    assert!(names.contains(&"orbit_graph_search"), "names: {names:?}");
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "tools/list must be name-sorted");

    // Snapshot guard: MCP tool input schema changes are breaking
    // (RELEASING.md), so the full advertised payload — the fixture-host tools
    // exercising Orbit's ToolSchema → JSON Schema conversion plus the entire
    // crate-owned orbit.graph.* surface — is pinned byte-for-byte.
    let snapshot_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT_RELATIVE_PATH);
    if std::env::var("ORBIT_MCP_UPDATE_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::create_dir_all(snapshot_path.parent().expect("snapshot dir"))
            .expect("create snapshot dir");
        let mut serialized = serde_json::to_string_pretty(&tools).expect("serialize snapshot");
        serialized.push('\n');
        std::fs::write(&snapshot_path, serialized).expect("write snapshot");
        return;
    }
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(&snapshot_path).unwrap_or_else(|err| {
            panic!(
                "cannot read {} ({err}); regenerate it with \
                 `ORBIT_MCP_UPDATE_SNAPSHOT=1 cargo test -p orbit-mcp --test mcp_wire_roundtrip`",
                snapshot_path.display()
            )
        }),
    )
    .expect("parse snapshot JSON");
    assert_eq!(
        tools, expected,
        "tools/list drifted from {SNAPSHOT_RELATIVE_PATH}. MCP tool schema changes are \
         BREAKING (see RELEASING.md). If the change is intentional, regenerate the snapshot \
         with `ORBIT_MCP_UPDATE_SNAPSHOT=1 cargo test -p orbit-mcp --test mcp_wire_roundtrip` \
         and call the release breaking."
    );
}

#[tokio::test]
async fn tools_call_round_trips_task_records_and_wraps_arrays() {
    let (mut client, host, _workspace) = start_fixture().await;
    client.initialize(None).await;

    // Create through the wire (advertised underscore name).
    let created = client
        .call_tool(
            "orbit_task_add",
            json!({ "title": "Wire round trip", "type": "feature", "tags": ["wire"] }),
        )
        .await;
    assert_eq!(created["isError"], false, "create failed: {created}");
    let created_record = structured(&created);
    let id = created_record["id"]
        .as_str()
        .expect("created id")
        .to_string();
    assert_eq!(created_record["title"], "Wire round trip");
    assert_eq!(created_record["type"], "feature");

    // Read back what was persisted in the temp store.
    let shown = client
        .call_tool("orbit_task_show", json!({ "id": id }))
        .await;
    assert_eq!(shown["isError"], false, "show failed: {shown}");
    assert_eq!(structured(&shown), created_record, "record must round-trip");

    // Array results are wrapped object-shaped for strict clients.
    let listed = client.call_tool("orbit_task_list", json!({})).await;
    assert_eq!(listed["isError"], false, "list failed: {listed}");
    let items = structured(&listed)["items"]
        .as_array()
        .expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0], *created_record);

    // The adapter must translate advertised names back to canonical dotted
    // names before host dispatch.
    let call_names: Vec<String> = host
        .recorded_calls()
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();
    assert_eq!(
        call_names,
        ["orbit.task.add", "orbit.task.show", "orbit.task.list"]
    );
}

#[tokio::test]
async fn initialize_meta_workspace_reaches_host_session_context() {
    // ADR-0181: clients announce the workspace through initialize `_meta`.
    // rmcp strips `_meta` from the params on the wire and re-delivers it via
    // the request context, so this only passes when the adapter reads the
    // transport-level meta — the exact regression this test pins.
    let (mut client, host, workspace) = start_fixture().await;
    let announced = workspace.path().to_str().expect("utf8 path").to_string();
    client
        .initialize(Some(json!({ "orbit": { "workspace": announced } })))
        .await;

    let created = client
        .call_tool("orbit_task_add", json!({ "title": "Session scoped" }))
        .await;
    assert_eq!(created["isError"], false, "create failed: {created}");
    assert_eq!(
        structured(&created)["session_workspace"],
        announced,
        "announced `_meta.orbit.workspace` must reach the host session context"
    );

    let calls = host.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].2.as_deref(), Some(announced.as_str()));
}

#[tokio::test]
async fn error_paths_return_structured_tool_errors_and_keep_the_server_alive() {
    let (mut client, _host, _workspace) = start_fixture().await;
    client.initialize(None).await;

    // Bad params: a host-side validation failure becomes an `isError` tool
    // result with a stable machine-readable code, not a JSON-RPC failure.
    let bad_params = client.call_tool("orbit_task_add", json!({})).await;
    assert_eq!(bad_params["isError"], true, "expected error: {bad_params}");
    assert_eq!(structured(&bad_params)["code"], "invalid_input");

    // Unknown tool: proper not-found classification, again without killing
    // the connection.
    let unknown = client.call_tool("orbit_task_frobnicate", json!({})).await;
    assert_eq!(unknown["isError"], true, "expected error: {unknown}");
    assert_eq!(structured(&unknown)["code"], "tool_not_found");

    // Unknown method: JSON-RPC method-not-found error.
    let response = client.request("orbit/no_such_method", Value::Null).await;
    assert_eq!(
        response["error"]["code"], -32601,
        "unknown methods must yield JSON-RPC method-not-found: {response}"
    );

    // The server must still serve requests after every error path above.
    let listed = client.call_tool("orbit_task_list", json!({})).await;
    assert_eq!(
        listed["isError"], false,
        "server wedged after errors: {listed}"
    );
}

#[tokio::test]
async fn graph_tools_dispatch_against_the_announced_workspace() {
    // The orbit.graph.* surface is served in-process by this crate; drive one
    // call end-to-end against a real temp worktree resolved from the
    // announced session workspace.
    let (mut client, _host, workspace) = start_fixture().await;
    std::fs::write(
        workspace.path().join("wire_probe.rs"),
        "pub fn wire_probe_symbol() {}\n",
    )
    .expect("write probe source");
    let announced = workspace.path().to_str().expect("utf8 path").to_string();
    client
        .initialize(Some(json!({ "orbit": { "workspace": announced } })))
        .await;

    let synced = client
        .call_tool("orbit_graph_sync", json!({ "full": true }))
        .await;
    assert_eq!(synced["isError"], false, "graph sync failed: {synced}");

    let found = client
        .call_tool(
            "orbit_graph_search",
            json!({ "query": "wire_probe_symbol", "kind": "symbol" }),
        )
        .await;
    assert_eq!(found["isError"], false, "graph search failed: {found}");
    let matches = structured(&found)["matches"]
        .as_array()
        .or_else(|| structured(&found)["items"].as_array())
        .unwrap_or_else(|| panic!("no matches array in {found}"));
    assert!(
        matches
            .iter()
            .any(|entry| entry.to_string().contains("wire_probe_symbol")),
        "expected wire_probe_symbol in graph search results: {found}"
    );
}
