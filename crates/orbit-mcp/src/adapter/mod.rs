//! The `dispatch` submodule owns MCP `tools/list` and `tools/call` wire framing and fans advertised tool names into host dispatch.
//! The `graph` submodule owns direct orbit-graph MCP wrappers that live in this long-running process.
//! The `structured` submodule owns the final `structuredContent` framing for strict MCP clients.
//! The `schema` submodule emits JSON input schemas from Orbit tool metadata.
//! The `name_map` submodule owns canonical-to-advertised tool name mapping and collision detection.
//! The `learning_sidecar` submodule owns learning reminder lookup, session admission, and response sidecar injection.

mod dispatch;
mod graph;
mod learning_sidecar;
mod name_map;
mod schema;
mod structured;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use orbit_common::types::{
    LearningInjectionCaps, LearningInjectionState, ToolSchema, ToolSessionContext,
};

use crate::McpHost;

/// An rmcp [`ServerHandler`] that delegates tool listing and tool execution to
/// an injected [`McpHost`].
///
/// Tools are enumerated on every `tools/list` request so late-registered or
/// newly-enabled tools become visible without a restart. Each `tools/call`
/// fans into the host's synchronous executor via [`tokio::task::spawn_blocking`]
/// because Orbit tool implementations issue blocking filesystem, git, and
/// SQLite calls.
///
/// Orbit's canonical tool names use dots (`orbit.task.add`), but several MCP
/// clients (Cursor, VS Code) reject names containing characters outside
/// `[a-z0-9_-]` and refuse to load the tool. The adapter sanitizes names by
/// replacing dots with underscores when advertising over MCP and translates
/// inbound `tools/call` names back to canonical form before dispatch. The
/// `name_map` is rebuilt from the host on every `tools/list` and
/// `tools/call` so dynamically-added tools cannot create stale or
/// ambiguous dispatch.
pub struct OrbitToolServer {
    host: Arc<dyn McpHost>,
    graph_tools: Arc<graph::GraphToolRegistry>,
    name_map: RwLock<HashMap<String, String>>,
    session_context: RwLock<ToolSessionContext>,
    learning_session_id: Option<String>,
    learning_caps: LearningInjectionCaps,
    learning_states: tokio::sync::Mutex<HashMap<String, LearningInjectionState>>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        let learning_session_id = std::env::var("ORBIT_SESSION_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let learning_caps = LearningInjectionCaps::from_env();
        let mut learning_states = HashMap::new();
        let key = learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string());
        learning_states.insert(key, LearningInjectionState::default());
        Self {
            host,
            graph_tools: Arc::new(graph::GraphToolRegistry::new()),
            name_map: RwLock::new(HashMap::new()),
            session_context: RwLock::new(ToolSessionContext::default()),
            learning_session_id,
            learning_caps,
            learning_states: tokio::sync::Mutex::new(learning_states),
        }
    }

    /// Return the canonical tool schemas that will be advertised over MCP.
    pub fn tool_schemas(&self) -> Vec<ToolSchema> {
        self.combined_tool_schemas()
    }

    #[cfg(test)]
    fn new_for_test(
        host: Arc<dyn McpHost>,
        learning_session_id: Option<String>,
        learning_caps: LearningInjectionCaps,
        initial_state: LearningInjectionState,
    ) -> Self {
        let key = learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string());
        let mut learning_states = HashMap::new();
        learning_states.insert(key, initial_state);
        Self {
            host,
            graph_tools: Arc::new(graph::GraphToolRegistry::new()),
            name_map: RwLock::new(HashMap::new()),
            session_context: RwLock::new(ToolSessionContext::default()),
            learning_session_id,
            learning_caps,
            learning_states: tokio::sync::Mutex::new(learning_states),
        }
    }
}

pub(super) const PROCESS_LEARNING_SESSION_KEY: &str = "__process__";
