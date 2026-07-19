//! End-to-end integration tests for the production MCP entry point.
//!
//! Each test initializes a real Orbit workspace in a temp dir, spawns the
//! actual `orbit mcp serve` binary with piped stdio — the exact transport MCP
//! clients use — and speaks raw newline-delimited JSON-RPC to it, crossing the
//! full serialize → dispatch → `RuntimeMcpHost` → `OrbitRuntime` → store →
//! serialize path.
//!
//! The `tools/list` snapshot is the breaking-change guard for the agent MCP
//! surface: per RELEASING.md, any tool input/output schema change is breaking.
#![allow(missing_docs)]
// ORB-00013: tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

/// Path of the checked-in `tools/list` snapshot, relative to the crate root.
const SNAPSHOT_RELATIVE_PATH: &str = "tests/snapshots/mcp_tools_list.json";

// ---------------------------------------------------------------------------
// Fixture: an initialized Orbit workspace fully isolated from ~/.orbit.
// ---------------------------------------------------------------------------

struct McpWorkspace {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

impl McpWorkspace {
    fn init() -> Self {
        Self::init_with_mode(None)
    }

    fn init_hub() -> Self {
        Self::init_with_mode(Some("hub"))
    }

    fn init_with_mode(host_mode: Option<&str>) -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&work).expect("create work");

        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&work)
            .output()
            .expect("initialize Git checkout");
        assert!(output.status.success(), "git init failed: {output:?}");

        let mut init_args = vec![
            "init",
            "--non-interactive",
            "--host-name",
            "mcp-roundtrip-host",
        ];
        if let Some(mode) = host_mode {
            init_args.extend(["--host-mode", mode]);
        }
        let output = Self::orbit_command(&work, &home)
            .args(init_args)
            .output()
            .expect("run global init");
        assert!(
            output.status.success(),
            "global init failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if host_mode == Some("hub") {
            let output = Self::orbit_command(&work, &home)
                .args(["host", "register"])
                .output()
                .expect("register hub identity");
            assert!(
                output.status.success(),
                "hub registration failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let output = Self::orbit_command(&work, &home)
            .args(["workspace", "init", "--name", "mcp-roundtrip"])
            .output()
            .expect("run workspace init");
        assert!(
            output.status.success(),
            "workspace init failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        Self {
            _temp: temp,
            home,
            work,
        }
    }

    fn orbit_command(work: &Path, home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orbit"));
        command
            .current_dir(work)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env_remove("ORBIT_ROOT")
            .env_remove("ORBIT_SESSION_ID")
            .env_remove("ORBIT_TASK_ID")
            .env_remove("ORBIT_RUN_ID")
            .env_remove("ORBIT_ACTIVITY_ID")
            .env_remove("ORBIT_STEP_INDEX")
            .env_remove("ORBIT_AGENT_NAME")
            .env_remove("ORBIT_AGENT_MODEL")
            .env_remove("ORBIT_MANAGED_RUN_CONTEXT")
            .env_remove("ORBIT_TASK_ACTOR_KIND");
        command
    }

    /// Spawn `orbit mcp serve`, run the MCP initialize handshake (announcing
    /// this workspace via `_meta.orbit.workspace`, ADR-0181), and return the
    /// connected client.
    fn serve(&self) -> McpClient {
        let child = Self::orbit_command(&self.work, &self.home)
            .args(["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn orbit mcp serve");
        let mut client = McpClient::new(child);

        let workspace = self.work.to_str().expect("utf8 workspace path");
        let response = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "orbit-mcp-roundtrip-test", "version": "0" },
                "_meta": { "orbit": { "workspace": workspace } },
            }),
        );
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["serverInfo"]["name"], "orbit-mcp");
        assert!(
            result["capabilities"]["tools"].is_object(),
            "tools capability missing: {result}"
        );
        client.notify("notifications/initialized");
        client
    }
}

// ---------------------------------------------------------------------------
// Minimal JSON-RPC stdio client. Responses may arrive out of order (the
// server fans tool calls into blocking workers), so match strictly by id.
// ---------------------------------------------------------------------------

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: i64,
}

impl McpClient {
    fn new(mut child: Child) -> Self {
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            next_id: 0,
        }
    }

    fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_string(message).expect("serialize message");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("write to server stdin");
        self.stdin.flush().expect("flush server stdin");
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(&message);
        loop {
            let line = self
                .lines
                .recv_timeout(RESPONSE_TIMEOUT)
                .unwrap_or_else(|err| {
                    panic!("no response to `{method}` (id {id}) within timeout: {err}")
                });
            if line.trim().is_empty() {
                continue;
            }
            let response: Value = serde_json::from_str(line.trim())
                .unwrap_or_else(|err| panic!("server emitted invalid JSON ({err}): {line}"));
            if response.get("id").and_then(Value::as_i64) == Some(id) {
                return response;
            }
        }
    }

    /// `tools/call` helper asserting a result envelope came back.
    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("tools/call `{name}` returned no result: {response}"))
    }

    /// `tools/call` that must succeed; returns the structured content.
    fn call_tool_ok(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call_tool(name, arguments);
        assert_eq!(result["isError"], false, "`{name}` failed: {result}");
        result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` returned no structuredContent: {result}"))
    }

    fn call_tool_with_meta_ok(&mut self, name: &str, arguments: Value, meta: Value) -> Value {
        let result = self.call_tool_with_meta(name, arguments, meta);
        assert_eq!(result["isError"], false, "`{name}` failed: {result}");
        result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` returned no structuredContent: {result}"))
    }

    fn call_tool_with_meta(&mut self, name: &str, arguments: Value, meta: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments, "_meta": meta }),
        );
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("tools/call `{name}` returned no result: {response}"))
    }

    fn call_tool_with_meta_err(&mut self, name: &str, arguments: Value, meta: Value) -> Value {
        let result = self.call_tool_with_meta(name, arguments, meta);
        assert_eq!(
            result["isError"], true,
            "`{name}` unexpectedly succeeded: {result}"
        );
        result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` returned no structuredContent: {result}"))
    }

    /// `tools/call` that must fail as a structured tool error; returns it.
    fn call_tool_err(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call_tool(name, arguments);
        assert_eq!(
            result["isError"], true,
            "`{name}` unexpectedly succeeded: {result}"
        );
        result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` returned no structuredContent: {result}"))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Closing stdin ends the stdio transport; give the server a moment to
        // exit cleanly, then make sure it is gone.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn mcp_serve_tools_list_matches_production_snapshot() {
    let workspace = McpWorkspace::init();
    let mut client = workspace.serve();

    let response = client.request("tools/list", Value::Null);
    let tools = response["result"]["tools"].clone();

    let names: Vec<&str> = tools
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "tools/list must be name-sorted");
    // Admin-only tools must never leak onto the agent MCP surface
    // (ORB-00289).
    for hidden in [
        "orbit_task_delete",
        "orbit_task_lint",
        "orbit_learning_prune",
    ] {
        assert!(!names.contains(&hidden), "{hidden} leaked into: {names:?}");
    }
    assert!(
        !names.contains(&"orbit_friction_update"),
        "operator-only tool leaked onto the default agent surface: {names:?}"
    );

    // Snapshot guard for the full production agent surface: names AND input
    // schemas. Any diff here is a breaking MCP schema change per RELEASING.md.
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
                 `ORBIT_MCP_UPDATE_SNAPSHOT=1 cargo test -p orbit-cli --test mcp_roundtrip`",
                snapshot_path.display()
            )
        }),
    )
    .expect("parse snapshot JSON");
    assert_eq!(
        tools, expected,
        "tools/list drifted from {SNAPSHOT_RELATIVE_PATH}. MCP tool schema changes are \
         BREAKING (see RELEASING.md). If the change is intentional, regenerate the snapshot \
         with `ORBIT_MCP_UPDATE_SNAPSHOT=1 cargo test -p orbit-cli --test mcp_roundtrip` and \
         call the release breaking."
    );
}

#[test]
fn mcp_serve_lists_canonical_agent_surface_outside_any_checkout() {
    let workspace = McpWorkspace::init();
    let scratch = workspace.home.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create non-workspace launch dir");
    let child = McpWorkspace::orbit_command(&scratch, &workspace.home)
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn checkout-independent MCP server");
    let mut client = McpClient::new(child);
    client.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "outside-checkout", "version": "0" }
        }),
    );
    client.notify("notifications/initialized");

    let listed = client.request("tools/list", Value::Null);
    let names = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"orbit_task_show"));
    assert!(names.contains(&"orbit_graph_search"));
    assert!(!names.contains(&"orbit_host_list"));

    let missing = client.call_tool_err("orbit_task_show", json!({ "id": "ORB-00001" }));
    assert!(missing["message"].as_str().is_some_and(|message| {
        message.contains("requires a workspace selector")
            && message.contains("_meta.orbit.workspace")
    }));
}

#[test]
fn hub_mcp_serve_is_checkoutless_frame_pure_and_audits_trusted_identity() {
    let workspace = McpWorkspace::init_hub();
    let global_root = workspace.home.join(".orbit");
    orbit_core::host_registry::host_registry_service_at(&global_root)
        .expect("hub registry service")
        .register_identity(
            &orbit_core::routines::HostIdentity {
                schema_version: orbit_core::routines::HOST_IDENTITY_SCHEMA_VERSION,
                machine_id: "hm_spoke".to_string(),
                host_id: "spoke".to_string(),
                mode: orbit_core::routines::HostMode::Spoke,
            },
            BTreeSet::new(),
        )
        .expect("register remote spoke fixture");
    let scratch = workspace.home.join("hub-scratch");
    std::fs::create_dir_all(&scratch).expect("create hub launch dir");
    let child = McpWorkspace::orbit_command(&scratch, &workspace.home)
        .args(["mcp", "serve", "--hub", "--capabilities", "agent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn checkoutless hub MCP server");
    let mut client = McpClient::new(child);
    let initialized = client.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "hub-roundtrip", "version": "0" },
            "_meta": { "orbit": { "workspace": "ws_mcp-roundtrip" } },
        }),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "orbit-mcp");
    assert!(
        initialized["result"]["instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.starts_with("orbit-hub-contract-v1:"))
    );
    client.notify("notifications/initialized");

    let listed = client.request("tools/list", Value::Null);
    let names = listed["result"]["tools"]
        .as_array()
        .expect("hub tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("hub tool name"))
        .collect::<Vec<_>>();
    assert!(
        names.contains(&"orbit_task_add"),
        "missing task surface: {names:?}"
    );
    assert!(!names.iter().any(|name| name.starts_with("orbit_graph_")));
    assert!(!names.contains(&"orbit_host_list"));
    assert!(!names.contains(&"orbit_friction_update"));

    let created = client.call_tool_with_meta_ok(
        "orbit_task_add",
        json!({
            "workspace": "ws_mcp-roundtrip",
            "title": "Checkoutless hub round trip",
            "description": "Created through fixed hub mode",
            "model": "codex"
        }),
        json!({
            "orbit": {
                "remote_session_context": {
                    "workspace": "ws_mcp-roundtrip",
                    "workspace_id": "ws_mcp-roundtrip",
                    "caller_machine_id": "hm_spoke",
                    "caller_host_id": "spoke",
                    "transport": "ssh-mcp",
                    "effective_capabilities": ["agent"],
                    "origin_session_id": "session-spoke",
                    "mcp_call_id": "mcall-remote-roundtrip"
                }
            }
        }),
    );
    assert_eq!(created["title"], "Checkoutless hub round trip");
    let graph_denied = client.call_tool_with_meta_err(
        "orbit_graph_search",
        json!({"workspace": "ws_mcp-roundtrip", "query": "must-not-run"}),
        json!({
            "orbit": {
                "remote_session_context": {
                    "workspace": "ws_mcp-roundtrip",
                    "workspace_id": "ws_mcp-roundtrip",
                    "caller_machine_id": "hm_spoke",
                    "caller_host_id": "spoke",
                    "transport": "ssh-mcp",
                    "effective_capabilities": ["agent"],
                    "origin_session_id": "session-spoke",
                    "mcp_call_id": "mcall-remote-graph-denied"
                }
            }
        }),
    );
    assert!(
        graph_denied["message"]
            .as_str()
            .is_some_and(|message| message.contains("placement is 'local-derived'")),
        "graph denial must retain canonical placement evidence: {graph_denied}"
    );
    let wire_payload = serde_json::to_string(&(listed, created)).expect("serialize wire payload");
    assert!(!wire_payload.contains(workspace.work.to_string_lossy().as_ref()));
    assert!(!wire_payload.contains(workspace.home.to_string_lossy().as_ref()));
    drop(client);

    let connection =
        Connection::open(workspace.home.join(".orbit/orbit.db")).expect("open hub audit store");
    let audit = connection
        .query_row(
            "SELECT COUNT(*), workspace_id, caller_machine_id, process_machine_id, transport, capabilities_json, mcp_call_id
             FROM audit_events WHERE tool_name = 'orbit.task.add'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .expect("hub task audit");
    assert_eq!(audit.0, 1, "one D2 audit row per accepted call");
    assert_eq!(audit.1.as_deref(), Some("ws_mcp-roundtrip"));
    assert_eq!(audit.2.as_deref(), Some("hm_spoke"));
    assert!(audit.3.as_deref().is_some_and(|id| id.starts_with("hm_")));
    assert_ne!(audit.2, audit.3, "caller and hub process stay distinct");
    assert_eq!(audit.4.as_deref(), Some("ssh-mcp"));
    assert_eq!(audit.5.as_deref(), Some("[\"agent\"]"));
    assert_eq!(audit.6.as_deref(), Some("mcall-remote-roundtrip"));

    let graph_audit = connection
        .query_row(
            "SELECT COUNT(*), status, mcp_call_id
             FROM audit_events WHERE tool_name = 'orbit.graph.search'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .expect("hub graph denial audit");
    assert_eq!(graph_audit.0, 1, "one D2 audit row per denied call");
    assert_eq!(graph_audit.1, "denied");
    assert!(
        graph_audit
            .2
            .as_deref()
            .is_some_and(|id| id.starts_with("mcall-"))
    );
}

#[test]
fn mcp_serve_round_trips_records_against_a_temp_workspace() {
    let workspace = McpWorkspace::init();
    let mut client = workspace.serve();

    // Task create → show → list. No explicit `workspace` argument anywhere:
    // the ambient session workspace announced via initialize `_meta` must be
    // applied (ADR-0181).
    let created = client.call_tool_ok(
        "orbit_task_add",
        json!({
            "title": "MCP round-trip task",
            "description": "Created over the MCP stdio transport",
            "type": "chore",
            "tags": ["mcp-roundtrip"],
        }),
    );
    let task_id = created["id"].as_str().expect("task id").to_string();
    assert_eq!(created["title"], "MCP round-trip task");
    assert_eq!(created["status"], "proposed");

    let shown = client.call_tool_ok("orbit_task_show", json!({ "id": task_id }));
    assert_eq!(shown["id"], json!(task_id));
    assert_eq!(shown["title"], "MCP round-trip task");
    assert_eq!(shown["description"], "Created over the MCP stdio transport");
    assert_eq!(shown["tags"], json!(["mcp-roundtrip"]));

    let listed = client.call_tool_ok("orbit_task_list", json!({}));
    let items = listed["items"].as_array().expect("task list items");
    assert!(
        items.iter().any(|task| task["id"] == json!(task_id)),
        "created task missing from list: {items:?}"
    );
    let listed_by_stable_id = client.call_tool_ok(
        "orbit_task_list",
        json!({ "workspace": "ws_mcp-roundtrip" }),
    );
    assert!(listed_by_stable_id["items"].as_array().is_some());

    // D3 enforces the non-hierarchical default agent capability. The agent may
    // create friction, but the operator-only triage mutation stays hidden and
    // returns the same typed denial when called by its canonical alias.
    let friction = client.call_tool_ok(
        "orbit_friction_add",
        json!({ "body": "MCP D2 exposure regression", "model": "codex" }),
    );
    let friction_id = friction["id"].as_str().expect("friction id").to_string();
    let denied = client.call_tool_err(
        "orbit_friction_update",
        json!({ "id": friction_id, "status": "triaged", "model": "codex" }),
    );
    assert_eq!(denied["code"], "invalid_input");
    assert!(
        denied["message"]
            .as_str()
            .is_some_and(|message| message.contains("capability denied"))
    );

    // Learning create → show → lexical federated search (no embedding
    // companion installed, so `orbit.search` must serve the lexical path).
    let learning = client.call_tool_ok(
        "orbit_learning_add",
        json!({ "summary": "mcp-roundtrip-learning literal marker" }),
    );
    let learning_id = learning["id"].as_str().expect("learning id").to_string();

    let learning_shown = client.call_tool_ok("orbit_learning_show", json!({ "id": learning_id }));
    assert_eq!(
        learning_shown["summary"], "mcp-roundtrip-learning literal marker",
        "learning must round-trip"
    );

    let search = client.call_tool_ok(
        "orbit_search",
        json!({ "query": "mcp-roundtrip-learning", "kind": "learning" }),
    );
    assert_eq!(search["mode"], "lexical");
    let hits = search["results"].as_array().expect("search results");
    assert!(
        hits.iter()
            .any(|hit| hit["id"] == json!(learning_id) && hit["source"] == json!("lexical")),
        "lexical search must find the learning: {hits:?}"
    );

    // ADR create → show.
    let adr = client.call_tool_ok(
        "orbit_adr_add",
        json!({
            "title": "MCP round-trip ADR",
            "body": "## Context\nMCP integration test.\n\n## Decision\nRound-trip an ADR over stdio.\n\n## Consequences\n- Guarded by integration tests.\n",
            "tags": ["mcp-roundtrip"],
        }),
    );
    let adr_id = adr["id"].as_str().expect("adr id").to_string();
    assert_eq!(adr["status"], "proposed");

    let adr_shown = client.call_tool_ok("orbit_adr_show", json!({ "id": adr_id }));
    assert_eq!(adr_shown["title"], "MCP round-trip ADR");
    assert_eq!(adr_shown["tags"], json!(["mcp-roundtrip"]));
}

#[test]
fn mcp_serve_error_paths_return_tool_errors_and_keep_serving() {
    let workspace = McpWorkspace::init();
    let mut client = workspace.serve();

    // Bad params: missing required field is a structured `invalid_input`
    // tool error, not a JSON-RPC failure or a crash.
    let bad_params = client.call_tool_err("orbit_task_add", json!({}));
    assert_eq!(bad_params["code"], "invalid_input");
    assert!(
        bad_params["message"]
            .as_str()
            .is_some_and(|message| message.contains("title")),
        "error should name the missing field: {bad_params}"
    );

    // Admin-only tool (reachable via CLI, deliberately unexposed over MCP):
    // preflight rejects it as tool-not-found.
    let unexposed = client.call_tool_err("orbit_task_delete", json!({ "id": "ORB-00000" }));
    assert_eq!(unexposed["code"], "tool_not_found");

    // Entirely unknown tool name.
    let unknown = client.call_tool_err("orbit_definitely_not_a_tool", json!({}));
    assert_eq!(unknown["code"], "tool_not_found");

    // Unknown JSON-RPC method: proper protocol-level error.
    let response = client.request("orbit/no_such_method", Value::Null);
    assert_eq!(
        response["error"]["code"], -32601,
        "unknown methods must yield JSON-RPC method-not-found: {response}"
    );

    // The server must still answer after every error path above.
    let listed = client.call_tool_ok("orbit_task_list", json!({}));
    assert!(listed["items"].is_array(), "server wedged: {listed}");
}

#[test]
fn mcp_graph_calls_persist_success_and_failure_audit_rows() {
    let workspace = McpWorkspace::init();
    let mut client = workspace.serve();

    client.call_tool_ok(
        "orbit_graph_search",
        json!({ "query": "mcp-audit-marker", "model": "codex" }),
    );
    let error = client.call_tool_err(
        "orbit_graph_show",
        json!({ "selector": "not-a-selector", "model": "codex" }),
    );
    assert_eq!(error["code"], "invalid_input");
    let unallowlisted = client.call_tool_err(
        "orbit.graph.pack",
        json!({ "selectors": ["file:src/lib.rs"], "model": "codex" }),
    );
    assert_eq!(unallowlisted["code"], "tool_not_found");
    drop(client);

    for (tool_name, status) in [
        ("orbit.graph.search", "success"),
        ("orbit.graph.show", "failure"),
        ("orbit.graph.pack", "denied"),
    ] {
        let output = McpWorkspace::orbit_command(&workspace.work, &workspace.home)
            .args(["audit", "list", "--tool", tool_name, "--json"])
            .output()
            .expect("query graph audit rows");
        assert!(
            output.status.success(),
            "audit list failed for {tool_name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Value = serde_json::from_slice(&output.stdout).expect("parse audit rows");
        let rows = rows.as_array().expect("audit row array");
        assert_eq!(rows.len(), 1, "exactly one audit row for {tool_name}");
        let row = &rows[0];
        assert_eq!(row["tool_name"], tool_name);
        assert_eq!(row["subcommand"], "run-mcp");
        assert_eq!(row["status"], status);
        assert_eq!(row["role"], "unverified");
        assert_eq!(row["transport"], "local");
        assert_eq!(row["effective_capabilities"], json!(["agent"]));
        assert!(row["workspace_id"].as_str().is_some());
        assert!(row["caller_machine_id"].as_str().is_some());
        assert_eq!(row["caller_machine_id"], row["process_machine_id"]);
        assert!(row["origin_session_id"].as_str().is_some());
        assert!(row["mcp_call_id"].as_str().is_some());
        assert!(row["duration_ms"].as_i64().is_some_and(|value| value >= 1));
    }
}
