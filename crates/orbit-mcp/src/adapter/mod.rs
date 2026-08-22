//! MCP tool dispatch, name translation, structured results, and input schemas.

mod dispatch;
mod name_map;
pub(crate) mod schema;
mod structured;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

use std::sync::{Arc, RwLock};

use orbit_common::observability::audit_id::audit_execution_id;
use orbit_types::tool::ToolSessionContext;

use crate::McpHost;

/// An rmcp server that delegates the complete tool surface to an [`McpHost`].
///
/// Definitions are read on every list and call so changes become visible
/// without a restart. Blocking host implementations run on a blocking worker.
/// Canonical dotted names are advertised with underscores and translated back
/// before dispatch.
pub struct OrbitToolServer {
    host: Arc<dyn McpHost>,
    /// The workspace this server process was launched for, when the launching
    /// configuration named one.
    ///
    /// A managed integration — what `orbit mcp init` and `orbit workspace init
    /// --mcp` generate — knows its workspace before any client connects, and
    /// most MCP clients cannot announce `_meta.orbit.workspace` at initialize
    /// at all. Keeping the launch binding here, separate from the mutable
    /// session context, lets `initialize` fall back to it instead of clearing
    /// the selector, while a re-initialize still resets to the launch value
    /// rather than inheriting the previous client's claim.
    launch_workspace: Option<String>,
    session_context: RwLock<ToolSessionContext>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        Self::new_with_context(host, ToolSessionContext::trusted_local(None, None, None))
    }

    pub fn new_with_context(
        host: Arc<dyn McpHost>,
        mut trusted_context: ToolSessionContext,
    ) -> Self {
        if trusted_context.origin_session_id.is_none() {
            trusted_context.origin_session_id = Some(audit_execution_id("mcp-session"));
        }
        trusted_context.trace_id = None;
        trusted_context.workspace = normalized_selector(trusted_context.workspace.as_deref());
        Self {
            host,
            launch_workspace: trusted_context.workspace.clone(),
            session_context: RwLock::new(trusted_context),
        }
    }
}

/// A selector is present only when it names something; blank is absent.
fn normalized_selector(selector: Option<&str>) -> Option<String> {
    selector
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
        .map(ToOwned::to_owned)
}
