#![deny(clippy::print_stderr, clippy::print_stdout)]
// Internal feature surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]
//! Thin remote MCP transport and discovery surface for Orbit.
//!
//! Registry and host identity behavior lives in `orbit-registry`; generic MCP
//! framing lives in `orbit-mcp`; Core remains authoritative for tool execution.

pub mod mcp;
pub use mcp::{
    McpServerIdentity, RemoteProxyArgs, canonical_mcp_tool_definitions, execute_discovery_tool,
    mcp_server_identity, safe_mcp_tool_names, serve_mcp_remote_proxy,
};
