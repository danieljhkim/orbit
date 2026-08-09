//! Per-session server construction for transports that serve more than one
//! client from a single listener.

use std::sync::Arc;

use orbit_common::types::ToolSessionContext;

use crate::{McpHost, McpServerComposition, OrbitToolServer};

/// Builds one independent [`OrbitToolServer`] per MCP session.
///
/// # Why sessions are not shared
///
/// The adapter's session state is mutable and is written during `initialize`:
/// the client's announced workspace selector is installed on the server
/// instance that handled the request, and every later `tools/list` and
/// `tools/call` on that instance reads it back. A stdio server has exactly one
/// client for its lifetime, so a single instance is correct there. A listener
/// does not: two clients sharing one instance would race on that selector, and
/// the loser would receive a successful response computed against the other
/// client's workspace. That is silent wrong data, not an error, so isolation is
/// established by construction rather than defended after the fact — a session
/// is handed a server nobody else holds.
///
/// # Capability
///
/// The effective capability set is whatever the caller placed in
/// `trusted_context`; this type neither supplies nor widens one. Nothing a
/// client sends can change it: `initialize` overwrites only the legacy
/// workspace selector, and the per-call resolver derives its context from the
/// session's trusted values.
///
/// Cloning is cheap — the host is shared behind an `Arc`, and a composition
/// holds `Arc`-shaped registrations — so building a session per connection is
/// an accept-path-appropriate cost.
#[derive(Clone)]
pub struct McpSessionFactory {
    host: Arc<dyn McpHost>,
    trusted_context: ToolSessionContext,
    composition: McpServerComposition,
}

impl McpSessionFactory {
    /// Capture the host, trusted session template, and composition every
    /// session is built from.
    pub fn new(
        host: Arc<dyn McpHost>,
        trusted_context: ToolSessionContext,
        composition: McpServerComposition,
    ) -> Self {
        Self {
            host,
            trusted_context,
            composition,
        }
    }

    /// Construct a server owned by exactly one session.
    ///
    /// The trusted template is copied rather than shared, and the correlation
    /// identifiers are cleared so the adapter mints a fresh origin session id
    /// per session: a listener-wide id would collapse concurrent clients into
    /// one audit identity.
    pub fn build_session(&self) -> OrbitToolServer {
        let mut context = self.trusted_context.clone();
        context.origin_session_id = None;
        context.mcp_call_id = None;
        OrbitToolServer::new_with_context_and_composition(
            Arc::clone(&self.host),
            context,
            self.composition.clone(),
        )
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
