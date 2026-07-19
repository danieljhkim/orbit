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
pub(crate) mod schema;
mod structured;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[cfg(test)]
use orbit_common::types::{LearningInjectionCaps, LearningInjectionState};
use orbit_common::types::{
    McpToolDefinition, McpToolPolicyError, ToolParam, ToolSessionContext, audit_execution_id,
};

use crate::{
    McpCallContextResolver, McpCustomRequestHandler, McpHost, McpInputSchema, McpResultDecorator,
    McpServerComposition, McpServerMetadata, McpToolExtension, McpToolExtensionRegistration,
};

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
    extensions: Vec<McpToolExtensionRegistration>,
    result_decorators: Vec<Arc<dyn McpResultDecorator>>,
    call_context_resolver: Arc<dyn McpCallContextResolver>,
    custom_request_handlers: Vec<Arc<dyn McpCustomRequestHandler>>,
    metadata: McpServerMetadata,
    name_map: RwLock<HashMap<String, String>>,
    session_context: RwLock<ToolSessionContext>,
    #[cfg(test)]
    learning_states_for_test:
        Option<Arc<tokio::sync::Mutex<std::collections::BTreeMap<String, LearningInjectionState>>>>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        Self::new_with_context(host, ToolSessionContext::trusted_local(None, None, None))
    }

    /// Construct a server with an explicit in-process extension composition.
    pub fn new_with_extensions(
        host: Arc<dyn McpHost>,
        extensions: Vec<McpToolExtensionRegistration>,
    ) -> Self {
        Self::new_with_context_and_extensions(
            host,
            ToolSessionContext::trusted_local(None, None, None),
            extensions,
        )
    }

    /// Construct a server from a complete generic MCP composition.
    pub fn new_with_composition(host: Arc<dyn McpHost>, composition: McpServerComposition) -> Self {
        Self::new_with_context_and_composition(
            host,
            ToolSessionContext::trusted_local(None, None, None),
            composition,
        )
    }

    pub fn new_with_context(host: Arc<dyn McpHost>, trusted_context: ToolSessionContext) -> Self {
        let extensions = default_extensions(&host);
        Self::new_with_context_and_extensions(host, trusted_context, extensions)
    }

    /// Construct a server with trusted context and an explicit in-process
    /// extension composition.
    pub fn new_with_context_and_extensions(
        host: Arc<dyn McpHost>,
        trusted_context: ToolSessionContext,
        extensions: Vec<McpToolExtensionRegistration>,
    ) -> Self {
        let composition = compatibility_composition(
            &host,
            extensions,
            Arc::new(learning_sidecar::LearningSidecarDecorator::from_env(
                Arc::clone(&host),
            )),
        );
        Self::new_with_context_and_composition(host, trusted_context, composition)
    }

    /// Construct a server with trusted context and a complete generic MCP
    /// composition. Unlike compatibility constructors, this installs only the
    /// extensions, decorators, handlers, resolver, and metadata supplied by
    /// the caller.
    pub fn new_with_context_and_composition(
        host: Arc<dyn McpHost>,
        mut trusted_context: ToolSessionContext,
        composition: McpServerComposition,
    ) -> Self {
        if trusted_context.origin_session_id.is_none() {
            trusted_context.origin_session_id = Some(audit_execution_id("mcp-session"));
        }
        trusted_context.mcp_call_id = None;
        let parts = composition.into_parts();
        Self {
            host,
            extensions: parts.tool_extensions,
            result_decorators: parts.result_decorators,
            call_context_resolver: parts.call_context_resolver,
            custom_request_handlers: parts.custom_request_handlers,
            metadata: parts.metadata,
            name_map: RwLock::new(HashMap::new()),
            session_context: RwLock::new(trusted_context),
            #[cfg(test)]
            learning_states_for_test: None,
        }
    }

    #[cfg(test)]
    fn new_for_test(
        host: Arc<dyn McpHost>,
        learning_session_id: Option<String>,
        learning_caps: LearningInjectionCaps,
        initial_state: LearningInjectionState,
    ) -> Self {
        let extensions = default_extensions(&host);
        let learning = Arc::new(learning_sidecar::LearningSidecarDecorator::new_for_test(
            Arc::clone(&host),
            learning_session_id,
            learning_caps,
            initial_state,
        ));
        let learning_states = learning.states();
        let composition = compatibility_composition(&host, extensions, learning);
        let mut trusted_context = ToolSessionContext::trusted_local(None, None, None);
        trusted_context.origin_session_id = Some(audit_execution_id("mcp-session"));
        let mut server = Self::new_with_context_and_composition(host, trusted_context, composition);
        server.learning_states_for_test = Some(learning_states);
        server
    }

    #[cfg(test)]
    async fn learning_state_for_test(&self, key: &str) -> Option<LearningInjectionState> {
        let states = self.learning_states_for_test.as_ref()?;
        states.lock().await.get(key).cloned()
    }
}

fn compatibility_composition(
    host: &Arc<dyn McpHost>,
    extensions: Vec<McpToolExtensionRegistration>,
    learning: Arc<dyn McpResultDecorator>,
) -> McpServerComposition {
    let mut composition = McpServerComposition::new()
        .with_tool_extensions(extensions)
        .with_result_decorator(learning)
        .with_call_context_resolver(Arc::new(dispatch::CompatibilityCallContextResolver::new(
            host.accepts_remote_session_context(),
        )));
    if host.accepts_remote_session_context() {
        composition = composition.with_custom_request_handler(Arc::new(
            dispatch::SpokeRegistrationHandler::new(Arc::clone(host)),
        ));
    }
    if let Some(instructions) = host.private_server_instructions() {
        composition =
            composition.with_metadata(McpServerMetadata::default().with_instructions(instructions));
    }
    composition
}

fn default_extensions(host: &Arc<dyn McpHost>) -> Vec<McpToolExtensionRegistration> {
    let graph: Arc<dyn McpToolExtension> = Arc::new(graph::GraphToolRegistry::new());
    let registration = if host.in_process_graph_tools_enabled() {
        McpToolExtensionRegistration::advertised(graph)
    } else {
        McpToolExtensionRegistration::recognition_only(graph)
    };
    vec![registration]
}

pub(super) const PROCESS_LEARNING_SESSION_KEY: &str = "__process__";

pub(crate) fn graph_tool_names() -> &'static [&'static str] {
    graph::GRAPH_TOOL_NAMES
}

pub(crate) fn graph_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
    graph::graph_tool_definitions()
}

pub(crate) fn encode_mcp_input_schema(tool_name: &str, params: &[ToolParam]) -> McpInputSchema {
    schema::build_input_schema(tool_name, params)
}

pub(crate) fn encode_mcp_input_schema_with_enum_values<F>(
    tool_name: &str,
    params: &[ToolParam],
    enum_values: F,
) -> McpInputSchema
where
    F: Fn(&str, &str) -> Option<&'static [&'static str]>,
{
    schema::build_input_schema_with_enum_values(tool_name, params, enum_values)
}
