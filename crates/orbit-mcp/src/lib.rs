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
//! The crate is a thin transport adapter between rmcp's server runtime and an
//! Orbit-supplied [`McpHost`]. `orbit-mcp` itself performs no tool dispatch,
//! no policy evaluation, and no audit logging — it delegates each
//! `tools/call` to the host. In the default `orbit-cli` wiring the host is
//! `RuntimeMcpHost`, which brackets every call with an audit boundary
//! (`audited_mcp_call`): the wrapper records a failure-status audit row when
//! preflight rejects an unknown / unexposed tool name, and otherwise dispatches
//! through `OrbitRuntime::execute_tool_command_dispatch` tagged with
//! `ToolEntryPoint::Mcp`, where the runtime persists a success-or-failure
//! audit row with the same identity-resolution rules and policy chain as the
//! CLI path. Audit rows from MCP calls carry `subcommand = "run-mcp"` so they
//! can be filtered apart from CLI tool runs (which carry `"run"`).
//!
//! # Role
//! Depends on `orbit-types` and `orbit-tools` only (for [`orbit_common::types::ToolSchema`]
//! and MCP-shape helpers). The CLI constructs a runtime-backed [`McpHost`] and
//! hands it to [`serve_stdio`]. No dependency on `orbit-core` is introduced.
//!
//! # Transport
//! Only stdio is supported in this cut. HTTP/SSE/streamable-http transports
//! are follow-up work once authentication is in scope.

mod adapter;
mod error;

use std::sync::Arc;

use orbit_common::types::{OrbitError, ToolSchema, ToolSessionContext};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use serde_json::Value;

pub use adapter::OrbitToolServer;

/// A pluggable back-end that satisfies MCP `tools/list` and `tools/call`
/// requests.
///
/// `list_tool_schemas` is expected to return only the tools the host wants
/// exposed — disabled tools should be filtered out here, not in the adapter.
/// `call_tool` must itself run whatever policy, audit, and sandboxing the host
/// wants applied; the adapter will never bypass it.
pub trait McpHost: Send + Sync + 'static {
    fn list_tool_schemas(&self) -> Vec<ToolSchema>;
    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError>;
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
