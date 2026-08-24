//! Live per-call probing and short-lived delivery to one configured destination.
//!
//! The mux answers `orbit.workspace.list` from what destinations say *now*, so
//! there is no health cache here and nothing is remembered between calls. A
//! routed workspace-scoped call opens one short-lived MCP session, confirms
//! the destination, and delivers that single `tools/call`. The client speaks
//! MCP over the same non-PTY SSH argv the v1 proxy uses.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use orbit_common::OrbitError;
use orbit_types::tool::mcp_advertised_tool_name;
use orbit_types::workspace::Workspace;
use serde_json::{Value, json};

use super::config::Destination;

/// How long one destination gets to complete the whole probe.
///
/// The budget covers SSH connection setup, the MCP handshake, and the
/// discovery call together, because a caller waiting on the list cannot tell
/// those phases apart and a per-phase budget would multiply the worst case by
/// the number of phases.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The MCP protocol revision this client negotiates. Pinned to the revision
/// Orbit's own server answers with, so a probe fails loudly on a real protocol
/// change rather than silently degrading.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// What one destination reported for this call.
#[derive(Debug, Clone)]
pub struct DestinationSnapshot {
    /// The `machine_id` the destination put on its own v1 envelope. The mux
    /// compares this against the operator's config pin.
    pub machine_id: String,
    pub workspaces: Vec<Workspace>,
}

/// One destination's live answer.
///
/// A trait rather than a concrete SSH call so the mux's projection, ordering,
/// routing, and failure handling are testable against fake destinations
/// without a reachable host.
pub trait DestinationProbe: Send + Sync {
    fn probe(&self, destination: &Destination) -> Result<DestinationSnapshot, OrbitError>;

    /// One short-lived session for a single routed `tools/call`.
    ///
    /// List and route never share a session or a health cache: a list that
    /// showed a workspace does not decide the next call's error.
    fn open_route(&self, destination: &Destination) -> Result<Box<dyn RoutedSession>, OrbitError>;
}

/// The MCP conversation opened for one routed call.
///
/// Snapshot, advertised tools, and the tool call share the session so mixed
/// version and health checks do not pay a second SSH handshake. The mux
/// classifies live errors *before* `call_tool`; a stale or unreachable
/// destination must not observe the call.
pub trait RoutedSession: Send {
    fn snapshot(&mut self) -> Result<DestinationSnapshot, OrbitError>;
    fn advertised_tools(&mut self) -> Result<Vec<String>, OrbitError>;
    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, OrbitError>;
}

/// The production probe: one short-lived SSH-hosted MCP session per call.
pub struct SshDestinationProbe {
    caller_machine_id: String,
    timeout: Duration,
}

impl SshDestinationProbe {
    pub fn new(caller_machine_id: String, timeout: Duration) -> Self {
        Self {
            caller_machine_id,
            timeout,
        }
    }
}

impl DestinationProbe for SshDestinationProbe {
    fn probe(&self, destination: &Destination) -> Result<DestinationSnapshot, OrbitError> {
        let mut session = self.start_session(destination)?;
        session.discover_workspaces()
    }

    fn open_route(&self, destination: &Destination) -> Result<Box<dyn RoutedSession>, OrbitError> {
        Ok(Box::new(SshRoutedSession {
            session: self.start_session(destination)?,
            snapshot: None,
            tools: None,
        }))
    }
}

impl SshDestinationProbe {
    fn start_session(&self, destination: &Destination) -> Result<DestinationSession, OrbitError> {
        let child = spawn_destination_session(destination, &self.caller_machine_id)?;
        // The session is one process; the guard ends it on every path,
        // including the timeout path where the child is still mid-answer.
        let mut session = DestinationSession::start(destination.clone(), child, self.timeout)?;
        session.handshake()?;
        Ok(session)
    }
}

/// Production routed session: one SSH child, several MCP requests, then drop.
struct SshRoutedSession {
    session: DestinationSession,
    snapshot: Option<DestinationSnapshot>,
    tools: Option<Vec<String>>,
}

impl RoutedSession for SshRoutedSession {
    fn snapshot(&mut self) -> Result<DestinationSnapshot, OrbitError> {
        if let Some(snapshot) = &self.snapshot {
            return Ok(snapshot.clone());
        }
        let snapshot = self.session.discover_workspaces()?;
        self.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn advertised_tools(&mut self) -> Result<Vec<String>, OrbitError> {
        if let Some(tools) = &self.tools {
            return Ok(tools.clone());
        }
        let tools = self.session.list_tools()?;
        self.tools = Some(tools.clone());
        Ok(tools)
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, OrbitError> {
        self.session.call_tool(name, arguments)
    }
}

/// Start the destination's MCP server over SSH with piped stdio.
///
/// `-T` and the remote argv are the v1 proxy's, so a destination sees exactly
/// the session shape it already supports. Only the stdio wiring differs: the
/// proxy inherits it for byte transparency, while the mux is itself the client
/// and must own both ends.
fn spawn_destination_session(
    destination: &Destination,
    caller_machine_id: &str,
) -> Result<Child, OrbitError> {
    Command::new("ssh")
        .arg("-T")
        .arg("--")
        .arg(&destination.ssh)
        .arg(crate::remote::remote_serve_command(caller_machine_id))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // The destination's logs are its own; folding them into this process's
        // stderr would interleave many hosts' output with no attribution.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| unreachable(destination, format!("could not start SSH: {error}")))
}

/// One MCP client session against a destination, bounded by a single deadline.
struct DestinationSession {
    destination: Destination,
    child: Child,
    stdin: std::process::ChildStdin,
    lines: Receiver<String>,
    deadline: Instant,
    next_id: i64,
}

impl DestinationSession {
    fn start(
        destination: Destination,
        mut child: Child,
        timeout: Duration,
    ) -> Result<Self, OrbitError> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| unreachable(&destination, "SSH session has no stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unreachable(&destination, "SSH session has no stdout".to_string()))?;
        // A reader thread is what makes the deadline real: a blocking read on
        // an unresponsive host cannot otherwise be abandoned, and the thread
        // ends on its own when the killed child closes the pipe.
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            destination,
            child,
            stdin,
            lines,
            deadline: Instant::now() + timeout,
            next_id: 0,
        })
    }

    fn handshake(&mut self) -> Result<(), OrbitError> {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "orbit-federated-mux",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;
        let negotiated = response["result"]["protocolVersion"].as_str();
        if negotiated != Some(PROTOCOL_VERSION) {
            return Err(unreachable(
                &self.destination,
                format!("destination negotiated MCP protocol {negotiated:?}"),
            ));
        }
        self.notify("notifications/initialized")
    }

    /// Call the destination's private federated discovery path and return its
    /// envelope. The public v1 list intentionally filters Invalid workspaces.
    fn discover_workspaces(&mut self) -> Result<DestinationSnapshot, OrbitError> {
        let response = self.request(
            "tools/call",
            json!({
                "name": crate::FEDERATED_DESTINATION_WORKSPACE_LIST_TOOL,
                "arguments": {},
            }),
        )?;
        let result = &response["result"];
        let content = &result["structuredContent"];
        if result["isError"].as_bool().unwrap_or(false) {
            // The destination's named code survives: wrapping it in a fresh
            // message here would leave the caller matching on prose.
            return Err(remote_tool_error(&self.destination, content));
        }
        snapshot_from_discovery_content(&self.destination, content)
    }

    fn list_tools(&mut self) -> Result<Vec<String>, OrbitError> {
        let response = self.request("tools/list", json!({}))?;
        let tools = response["result"]["tools"].as_array().ok_or_else(|| {
            unreachable(
                &self.destination,
                "tools/list answer carried no tools array".to_string(),
            )
        })?;
        Ok(tools
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(ToOwned::to_owned))
            .collect())
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, OrbitError> {
        let response = self.request(
            "tools/call",
            json!({
                "name": mcp_advertised_tool_name(name),
                "arguments": arguments,
            }),
        )?;
        let result = &response["result"];
        let content = &result["structuredContent"];
        if result["isError"].as_bool().unwrap_or(false) {
            // Named destination codes such as `capability_refused` must survive
            // as `RemoteTool`, not be wrapped into `execution_failed`.
            return Err(remote_tool_error(&self.destination, content));
        }
        if content.is_null() {
            return Ok(json!({}));
        }
        Ok(content.clone())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, OrbitError> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.await_response(method, id)
    }

    fn notify(&mut self, method: &str) -> Result<(), OrbitError> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }))
    }

    fn send(&mut self, message: &Value) -> Result<(), OrbitError> {
        let mut line = serde_json::to_string(message).map_err(|error| {
            OrbitError::Execution(format!("serialize federated probe request: {error}"))
        })?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|error| unreachable(&self.destination, format!("write failed: {error}")))
    }

    /// Read until the response with this id arrives or the deadline passes.
    /// Matching strictly by id keeps a server-initiated message or an
    /// out-of-order answer from being read as this request's result.
    fn await_response(&mut self, method: &str, id: i64) -> Result<Value, OrbitError> {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(unreachable(
                        &self.destination,
                        format!("timed out waiting for '{method}'"),
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(unreachable(
                        &self.destination,
                        format!("session ended before answering '{method}'"),
                    ));
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = match serde_json::from_str(line.trim()) {
                Ok(message) => message,
                Err(error) => {
                    return Err(unreachable(
                        &self.destination,
                        format!("emitted invalid JSON: {error}"),
                    ));
                }
            };
            if message.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(unreachable(
                        &self.destination,
                        format!("'{method}' failed: {error}"),
                    ));
                }
                return Ok(message);
            }
        }
    }
}

pub(crate) fn snapshot_from_discovery_content(
    destination: &Destination,
    content: &Value,
) -> Result<DestinationSnapshot, OrbitError> {
    let machine_id = content["machine_id"]
        .as_str()
        .ok_or_else(|| {
            unreachable(
                destination,
                "discovery answer carried no machine_id".to_string(),
            )
        })?
        .to_string();
    let workspaces: Vec<Workspace> = serde_json::from_value(content["workspaces"].clone())
        .map_err(|error| {
            unreachable(
                destination,
                format!("discovery answer was not a workspace list: {error}"),
            )
        })?;
    Ok(DestinationSnapshot {
        machine_id,
        workspaces,
    })
}

impl Drop for DestinationSession {
    fn drop(&mut self) {
        if let Err(error) = self.child.kill() {
            tracing::debug!(
                machine_id = %self.destination.machine_id,
                %error,
                "federated probe session was already gone"
            );
        }
        // Reap it: an unwaited SSH child would linger as a zombie for the life
        // of this long-running server process, once per destination per call.
        let _ = self.child.wait();
    }
}

fn unreachable(destination: &Destination, reason: String) -> OrbitError {
    OrbitError::UnreachableDestination(format!("{}: {reason}", destination.machine_id))
}

/// Preserve a destination's structured tool error as-is.
fn remote_tool_error(destination: &Destination, payload: &Value) -> OrbitError {
    let code = payload["code"]
        .as_str()
        .unwrap_or("execution_failed")
        .to_string();
    let message = payload["message"].as_str().unwrap_or_default();
    OrbitError::RemoteTool {
        code,
        message: format!("{}: {message}", destination.machine_id),
        payload: payload.clone(),
    }
}
