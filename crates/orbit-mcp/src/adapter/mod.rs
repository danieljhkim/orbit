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

use orbit_common::types::{ToolSessionContext, audit_execution_id};

use crate::McpHost;

/// An rmcp server that delegates the complete tool surface to an [`McpHost`].
///
/// Definitions are read on every list and call so changes become visible
/// without a restart. Blocking host implementations run on a blocking worker.
/// Canonical dotted names are advertised with underscores and translated back
/// before dispatch.
pub struct OrbitToolServer {
    host: Arc<dyn McpHost>,
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
        Self {
            host,
            session_context: RwLock::new(trusted_context),
        }
    }
}
