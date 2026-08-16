use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_common::observability::audit_id::audit_execution_id;
#[cfg(test)]
use orbit_types::tool::ToolSchema;
use orbit_types::tool::{McpToolDefinition, ToolSessionContext};
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeRequestParams,
    InitializeResult, ListToolsResult, Meta, PaginatedRequestParams, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};

use super::OrbitToolServer;
use super::name_map::{ToolNameCollision, build_name_map};
use super::schema::{ensure_workspace_selector, schema_to_tool};
use super::structured::mcp_structured_content;
use crate::error::tool_error_result;

impl OrbitToolServer {
    /// Return the host's complete exposed surface after validating only the
    /// canonical and advertised names the MCP kernel itself owns.
    pub(super) fn tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let definitions = self.host.list_mcp_tool_definitions()?;
        let schemas = definitions
            .iter()
            .map(|definition| definition.schema.clone())
            .collect::<Vec<_>>();
        if let Some(schema) = schemas.iter().find(|schema| schema.name.trim().is_empty()) {
            return Err(OrbitError::InvalidInput(format!(
                "canonical MCP tool name must not be empty: {:?}",
                schema.name
            )));
        }
        build_name_map(&schemas).map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        Ok(definitions)
    }

    #[cfg(test)]
    pub(super) fn tool_schemas(&self) -> Result<Vec<ToolSchema>, OrbitError> {
        Ok(self
            .tool_definitions()?
            .into_iter()
            .map(|definition| definition.schema)
            .collect())
    }

    /// Resolve the advertised wire input schema for one canonical definition.
    pub(super) fn input_schema_for(
        &self,
        definition: &McpToolDefinition,
    ) -> Result<Map<String, Value>, OrbitError> {
        let mut schema = super::schema::build_input_schema(
            &definition.schema.name,
            &definition.schema.parameters,
        );
        ensure_workspace_selector(&mut schema, definition);
        Ok(schema)
    }

    pub(crate) fn replace_session_context(&self, session_context: ToolSessionContext) {
        if let Ok(mut guard) = self.session_context.write() {
            *guard = session_context;
        }
    }

    pub(crate) fn session_context(&self) -> ToolSessionContext {
        self.session_context
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Clone the trusted session envelope and mint exactly one trace for this
    /// call. The context is not written back, so concurrent calls never share a
    /// trace and initialize-owned session state remains unchanged.
    fn context_for_tool_call(&self) -> ToolSessionContext {
        let mut context = self.session_context();
        context.trace_id = Some(audit_execution_id("trace"));
        context
    }

    pub(super) fn canonical_name(&self, advertised: &str) -> Result<String, McpError> {
        let schemas = self
            .tool_definitions()
            .map(|definitions| {
                definitions
                    .into_iter()
                    .map(|definition| definition.schema)
                    .collect::<Vec<_>>()
            })
            .map_err(invalid_definitions_mcp_error)?;
        let map = build_name_map(&schemas).map_err(ToolNameCollision::into_mcp_error)?;
        Ok(map
            .get(advertised)
            .cloned()
            .unwrap_or_else(|| advertised.to_string()))
    }

    #[cfg(test)]
    pub(super) async fn call_tool_request(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_tool_call(request).await
    }

    async fn dispatch_tool_call(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        let call_context = self.context_for_tool_call();
        let canonical = self.canonical_name(request.name.as_ref())?;
        let input = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let host = Arc::clone(&self.host);
        let execution_name = canonical.clone();
        let result = tokio::task::spawn_blocking(move || {
            host.call_tool(&execution_name, input, call_context)
        })
        .await;

        match result {
            Ok(Ok(value)) => Ok(CallToolResult::structured(mcp_structured_content(value))),
            Ok(Err(error)) => Ok(tool_error_result(&error)),
            Err(join_error) => {
                let error = OrbitError::Execution(format!(
                    "tool '{canonical}' worker panicked or was cancelled: {join_error}"
                ));
                Ok(tool_error_result(&error))
            }
        }
    }
}

impl ServerHandler for OrbitToolServer {
    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        let announced = session_context_from_initialize(&request, &context.meta);
        let mut trusted = self.session_context();
        // Initialize controls only the legacy workspace selector. Caller,
        // process, transport, and correlation facts remain server-owned.
        trusted.workspace = announced.workspace;
        trusted.trace_id = None;
        self.replace_session_context(trusted);
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        std::future::ready(Ok(self.get_info()))
    }

    fn get_info(&self) -> ServerInfo {
        let implementation = Implementation::new("orbit-mcp", env!("CARGO_PKG_VERSION"));
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(
                "Orbit tool registry exposed over MCP. Call tools/list to discover available \
                 operations; each tool advertises its own input schema.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut definitions = self
            .tool_definitions()
            .map_err(invalid_definitions_mcp_error)?;
        definitions.sort_by(|left, right| left.schema.name.cmp(&right.schema.name));
        let tools = definitions
            .into_iter()
            .map(|definition| {
                let input_schema = self
                    .input_schema_for(&definition)
                    .map_err(invalid_definitions_mcp_error)?;
                Ok(schema_to_tool(definition.schema, input_schema))
            })
            .collect::<Result<Vec<_>, McpError>>()?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_tool_call(request).await
    }
}

fn invalid_definitions_mcp_error(error: OrbitError) -> McpError {
    McpError::internal_error(
        format!("invalid canonical MCP tool definitions: {error}"),
        Some(serde_json::json!({ "code": "invalid_tool_definitions" })),
    )
}

pub(super) fn session_context_from_initialize(
    request: &InitializeRequestParams,
    transport_meta: &Meta,
) -> ToolSessionContext {
    // rmcp moves wire `_meta` to the request context. Prefer params-level meta
    // for in-process callers, then fall back to the transport-level value.
    let workspace = workspace_from_meta(request.meta.as_ref().map(|meta| &meta.0))
        .or_else(|| workspace_from_meta(Some(&transport_meta.0)));

    ToolSessionContext {
        workspace,
        ..ToolSessionContext::default()
    }
}

fn workspace_from_meta(meta: Option<&rmcp::model::JsonObject>) -> Option<String> {
    meta.and_then(|meta| {
        meta.get("orbit")
            .and_then(|orbit| orbit.get("workspace"))
            .or_else(|| meta.get("orbit.workspace"))
            .and_then(Value::as_str)
    })
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToOwned::to_owned)
}
