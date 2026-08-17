//! Thin SSH transport and remote-machine MCP support data.

mod discovery;
mod identity;
mod proxy;
mod surface;

#[cfg(test)]
mod tests;

pub use self::discovery::execute_discovery_tool;
pub use self::identity::{McpServerIdentity, McpSessionAuthority, mcp_server_identity};
pub use self::proxy::{RemoteProxyArgs, serve_mcp_remote_proxy};
pub use self::surface::{canonical_mcp_tool_definitions, safe_mcp_tool_names};
