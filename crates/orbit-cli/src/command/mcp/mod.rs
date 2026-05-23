//! `orbit mcp` — MCP client integration and server.
//!
//! `orbit mcp init/remove` manages local client integration for Claude Code,
//! Codex, Gemini, and Grok. `orbit mcp serve` serves the Orbit tool surface over
//! MCP so external clients can discover and invoke Orbit operations with typed
//! JSON schemas.

mod command;
mod host;
mod setup;

pub use command::{McpCommand, McpSubcommand};
pub(crate) use host::{ORBIT_MCP_SERVER_ID, safe_mcp_tool_names};
#[allow(unused_imports)]
pub(crate) use setup::init_auto_for_workspace;

#[cfg(test)]
mod tests;
