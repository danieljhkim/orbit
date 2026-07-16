#![deny(clippy::print_stderr, clippy::print_stdout)]
// ORB-00004: legacy MCP adapter surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! MCP (Model Context Protocol) server that exposes an Orbit tool surface to
//! any MCP-capable client.
//!
//! The crate is primarily a thin transport adapter between rmcp's server
//! runtime and an Orbit-supplied [`McpHost`]. Most tool dispatch, policy
//! evaluation, and audit logging is delegated to the host. The implementation
//! of the `orbit.graph.*` surface is backed by `orbit-graph` in-process so a
//! long-running MCP server can reuse one graph handle per worktree and apply
//! the MCP watcher-backed sync policy. Its policy and audit boundary still
//! belong to the host. In the default `orbit-cli` wiring the host is
//! `RuntimeMcpHost`, which applies the safe-surface preflight and brackets both
//! registry-backed and in-process calls with OrbitRuntime's audit boundary,
//! tagged with `ToolEntryPoint::Mcp`. The runtime persists a success-or-failure
//! audit row with the same identity-resolution rules as the CLI path. Audit
//! rows from MCP calls carry `subcommand = "run-mcp"` so they can be filtered
//! apart from CLI tool runs (which carry `"run"`).
//!
//! # Role
//! Depends on `orbit-common`, `orbit-graph`, and `orbit-graph-extract`. The
//! CLI constructs a runtime-backed [`McpHost`] and hands it to [`serve_stdio`].
//! No dependency on `orbit-core` is introduced.
//!
//! # Transport
//! Only stdio is supported in this cut. HTTP/SSE/streamable-http transports
//! are follow-up work once authentication is in scope.

mod adapter;
mod error;

use std::sync::Arc;

use orbit_common::types::{
    LearningInjectionState, NotFoundKind, OrbitError, ToolSchema, ToolSessionContext,
};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use serde_json::Value;

pub use adapter::OrbitToolServer;

/// Canonical names implemented by the in-process graph adapter.
pub fn graph_tool_names() -> &'static [&'static str] {
    adapter::graph_tool_names()
}

/// A pluggable back-end that satisfies MCP `tools/list` and `tools/call`
/// requests.
///
/// `list_tool_schemas` is expected to return only the tools the host wants
/// exposed — disabled tools should be filtered out here, not in the adapter.
/// `call_tool` and `call_in_process_tool` must run whatever policy, audit, and
/// sandboxing the host wants applied; the adapter never invokes an in-process
/// implementation without first crossing the latter host seam.
pub trait McpHost: Send + Sync + 'static {
    fn list_tool_schemas(&self) -> Vec<ToolSchema>;
    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError>;

    /// Apply host policy and auditing around a tool implemented inside this
    /// adapter. The secure default rejects the call; hosts must explicitly
    /// admit adapter-owned tools and invoke `dispatch` inside their boundary.
    fn call_in_process_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
        _dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }

    /// L-0043: sidecar internals use host extensions so runtime-backed MCP
    /// hosts can keep the client safe surface narrow.
    fn learning_candidates_for_path(
        &self,
        path: &str,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.call_tool(
            "orbit.learning.list",
            serde_json::json!({ "path": path }),
            session_context,
        )
    }

    fn get_session_learning_state(
        &self,
        _session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        Ok(None)
    }

    fn upsert_session_learning_state(
        &self,
        _session_id: &str,
        _state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        Ok(())
    }
}

/// Serve the given [`McpHost`] over an MCP stdio transport.
///
/// Runs until the client disconnects or the server encounters a fatal
/// transport error. The function is async and expects to be driven by a tokio
/// runtime (see `tokio::runtime::Runtime::block_on`).
pub async fn serve_stdio(host: Arc<dyn McpHost>) -> Result<(), OrbitError> {
    let server = OrbitToolServer::new(host);
    let running = server
        .serve(stdio())
        .await
        .map_err(|err| OrbitError::Execution(format!("mcp serve_stdio start: {err}")))?;
    running
        .waiting()
        .await
        .map_err(|err| OrbitError::Execution(format!("mcp serve_stdio wait: {err}")))?;
    Ok(())
}
