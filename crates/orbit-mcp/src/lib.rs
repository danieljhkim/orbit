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
mod client;
mod error;
mod hub_contract;

use std::sync::Arc;

use orbit_common::types::{
    LearningInjectionState, McpToolDefinition, McpToolPolicyError, NotFoundKind, OrbitError,
    SpokeRegistrationRequestV1, SpokeRegistrationResultV1, ToolSessionContext,
};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use serde_json::Value;

pub use adapter::OrbitToolServer;
pub use client::{HubClientExpectation, OrbitMcpClient, validate_remote_call_context};
pub use hub_contract::{
    CANONICAL_MCP_REGISTRY_REVISION, HUB_CONTRACT_INSTRUCTIONS_PREFIX, HUB_SCHEMA_DOMAIN,
    HubServerContractV1, MCP_CONTRACT_REVISION, canonical_hub_schema_bytes, hub_schema_digest,
};

/// Canonical names implemented by the in-process graph adapter.
pub fn graph_tool_names() -> &'static [&'static str] {
    adapter::graph_tool_names()
}

/// Workspace-independent source for the graph adapter's schema-adjacent policies.
pub fn graph_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
    adapter::graph_mcp_tool_definitions()
}

/// A pluggable back-end that satisfies MCP `tools/list` and `tools/call`
/// requests.
///
/// `list_mcp_tool_definitions` returns only the tools the host wants exposed,
/// with schema and policy already paired at their canonical definition site.
/// Disabled tools should be filtered out here, not in the adapter.
/// `call_tool` and `call_in_process_tool` must run whatever policy, audit, and
/// sandboxing the host wants applied; the adapter never invokes an in-process
/// implementation without first crossing the latter host seam.
pub trait McpHost: Send + Sync + 'static {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError>;

    /// Whether the adapter may install its local-checkout graph implementations.
    /// The ordinary broker enables them by default. A checkoutless hub server
    /// disables them structurally so adapter-owned local-derived tools cannot
    /// be merged back into its hub-only surface.
    fn in_process_graph_tools_enabled(&self) -> bool {
        true
    }

    /// Private initialize instructions used only by a fixed hub transport.
    /// Ordinary MCP servers keep the human-readable default.
    fn private_server_instructions(&self) -> Option<String> {
        None
    }

    /// Whether connector-owned per-call metadata may replace the local
    /// session context. Only the explicit checkoutless hub host enables this.
    fn accepts_remote_session_context(&self) -> bool {
        false
    }

    /// Handle the one connector-private spoke bootstrap request.
    ///
    /// `None` is the secure default and makes the adapter return JSON-RPC
    /// `METHOD_NOT_FOUND`. Only the explicit checkoutless hub host overrides
    /// this seam. It is never represented by an MCP tool definition.
    fn private_register_spoke(
        &self,
        _request: SpokeRegistrationRequestV1,
        _session_context: ToolSessionContext,
    ) -> Option<Result<SpokeRegistrationResultV1, OrbitError>> {
        None
    }

    /// Validate trusted call identity before the adapter resolves a canonical
    /// tool definition. Hub hosts use this to reject unknown/retired callers
    /// before any registry discovery or domain dispatch.
    fn preflight_tool_call(
        &self,
        _name: &str,
        _session_context: &ToolSessionContext,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError>;

    /// Observe a transport-level capability denial. The default preserves the
    /// denial unchanged; audited brokers may record it without executing the
    /// tool or constructing a workspace runtime.
    fn reject_tool_call(
        &self,
        _name: &str,
        _input: &Value,
        _session_context: &ToolSessionContext,
        denial: OrbitError,
    ) -> OrbitError {
        denial
    }

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
    serve_stdio_with_context(host, ToolSessionContext::trusted_local(None, None, None)).await
}

/// Serve stdio MCP with adapter-validated trusted context. The external
/// initialize payload can replace only the legacy `workspace` selector.
pub async fn serve_stdio_with_context(
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
) -> Result<(), OrbitError> {
    let server = OrbitToolServer::new_with_context(host, trusted_context);
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
