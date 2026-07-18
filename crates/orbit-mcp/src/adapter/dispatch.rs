use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{
    McpToolDefinition, NotFoundKind, OrbitError, ToolSchema, ToolSessionContext,
    mcp_advertised_tool_name,
};
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};

use super::OrbitToolServer;
use super::name_map::{ToolNameCollision, build_name_map};
use super::schema::schema_to_tool;
use super::structured::mcp_structured_content;
use crate::error::tool_error_result;

impl OrbitToolServer {
    pub(super) fn combined_tool_definitions(&self) -> Vec<McpToolDefinition> {
        let mut definitions = self
            .host
            .list_tool_schemas()
            .into_iter()
            .filter(|schema| !self.graph_tools.is_graph_tool(&schema.name))
            .filter_map(|schema| {
                let policy = self.host.mcp_tool_policy(&schema.name)?;
                McpToolDefinition::new(schema, policy).ok()
            })
            .collect::<Vec<_>>();
        // ORB-00391: the v1 orbit-knowledge graph builtins were decommissioned,
        // so the in-process orbit-graph (v2) adapter owns its known graph
        // names and their local-derived policies.
        definitions.extend(self.graph_tools.definitions());
        definitions
    }

    pub(super) fn combined_tool_schemas(&self) -> Vec<ToolSchema> {
        self.combined_tool_definitions()
            .into_iter()
            .map(|definition| definition.schema)
            .collect()
    }

    // pub(super) visibility widened from private so that adapter::tests (sibling under adapter)
    // can exercise the name-mapping and canonical-name logic after collapsing the nested
    // tests/ anti-pattern. These remain internal to the adapter module; not part of the
    // crate-public API. See ORB-00242.
    pub(super) fn refresh_name_map(&self, schemas: &[ToolSchema]) -> Result<(), ToolNameCollision> {
        let map = match build_name_map(schemas) {
            Ok(map) => map,
            Err(err) => {
                self.clear_name_map();
                return Err(err);
            }
        };
        self.replace_name_map(map);
        Ok(())
    }

    pub(super) fn replace_name_map(&self, map: HashMap<String, String>) {
        if let Ok(mut guard) = self.name_map.write() {
            *guard = map;
        }
    }

    pub(super) fn clear_name_map(&self) {
        if let Ok(mut guard) = self.name_map.write() {
            guard.clear();
        }
    }

    pub(super) fn replace_session_context(&self, session_context: ToolSessionContext) {
        if let Ok(mut guard) = self.session_context.write() {
            *guard = session_context;
        }
    }

    pub(super) fn session_context(&self) -> ToolSessionContext {
        self.session_context
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(super) fn canonical_name(&self, advertised: &str) -> Result<String, ToolNameCollision> {
        let schemas = self.combined_tool_schemas();
        let map = match build_name_map(&schemas) {
            Ok(map) => map,
            Err(err) => {
                self.clear_name_map();
                return Err(err);
            }
        };
        let resolved = map.get(advertised).cloned();
        self.replace_name_map(map);
        Ok(resolved.unwrap_or_else(|| advertised.to_string()))
    }

    pub(super) async fn call_tool_request(
        &self,
        req: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        let inbound = req.name.to_string();
        let canonical = self
            .canonical_name(&inbound)
            .map_err(ToolNameCollision::into_mcp_error)?;
        let has_valid_definition = self
            .combined_tool_schemas()
            .iter()
            .any(|schema| schema.name == canonical);
        // L-0093: reject declared policy drift here, but let truly unknown names
        // reach the host preflight because it owns rejected-call audit insertion.
        let declared_without_valid_policy = !has_valid_definition
            && (self.host.list_tool_schemas().iter().any(|schema| {
                schema.name == canonical || mcp_advertised_tool_name(&schema.name) == inbound
            }) || self.graph_tools.is_graph_tool(&canonical)
                || super::graph::GRAPH_TOOL_NAMES
                    .iter()
                    .any(|name| mcp_advertised_tool_name(name) == inbound));
        if declared_without_valid_policy {
            return Ok(tool_error_result(&OrbitError::not_found(
                NotFoundKind::Tool,
                canonical,
            )));
        }
        let input = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let host = Arc::clone(&self.host);
        let graph_tools = Arc::clone(&self.graph_tools);
        let exec_name = canonical.clone();
        let session_context = self.session_context();
        let input_for_learning = input.clone();
        // Dispatch recognition is deliberately independent of host schemas.
        // Re-exposing a host graph schema must not make
        // adapter-owned graph calls bypass the host's policy/audit seam.
        let graph_tool = self.graph_tools.is_graph_tool(&canonical);
        let join = tokio::task::spawn_blocking(move || {
            if graph_tool {
                let graph_name = exec_name.clone();
                let mut dispatch = move |input, session_context| {
                    graph_tools.call_tool(&graph_name, input, session_context)
                };
                host.call_in_process_tool(&exec_name, input, session_context, &mut dispatch)
            } else {
                host.call_tool(&exec_name, input, session_context)
            }
        })
        .await;

        match join {
            Ok(Ok(value)) => {
                let value = self
                    .maybe_attach_learning_sidecar(&canonical, input_for_learning, value)
                    .await?;
                Ok(CallToolResult::structured(mcp_structured_content(value)))
            }
            Ok(Err(orbit_err)) => {
                if graph_tool {
                    tracing::warn!(
                        target: "orbit.mcp.graph",
                        tool = %canonical,
                        error = %orbit_err,
                        "graph tool call failed"
                    );
                }
                Ok(tool_error_result(&orbit_err))
            }
            Err(join_err) => {
                let err = OrbitError::Execution(format!(
                    "tool '{canonical}' worker panicked or was cancelled: {join_err}"
                ));
                Ok(tool_error_result(&err))
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
        self.replace_session_context(session_context_from_initialize(&request, &context.meta));
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
                 task, graph, state, and review operations; each tool advertises its own input \
                 schema.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut schemas = self.combined_tool_schemas();
        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        self.refresh_name_map(&schemas)
            .map_err(ToolNameCollision::into_mcp_error)?;
        let tools = schemas.into_iter().map(schema_to_tool).collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool_request(req).await
    }
}

pub(super) fn session_context_from_initialize(
    request: &InitializeRequestParams,
    transport_meta: &rmcp::model::Meta,
) -> ToolSessionContext {
    // ADR-0181: clients deliberately announce workspace through initialize `_meta`.
    //
    // rmcp's wire deserializer strips `_meta` out of the request params and
    // parks it on the request extensions (surfaced here as the request
    // context's `meta`), so `request.meta` is always `None` for requests that
    // crossed a real transport. Check the params-level field first for
    // callers that construct `InitializeRequestParams` in-process, then fall
    // back to the transport-level meta. Covered by the crate integration
    // test `initialize_meta_workspace_reaches_host_session_context`.
    let workspace = workspace_from_meta(request.meta.as_ref().map(|meta| &meta.0))
        .or_else(|| workspace_from_meta(Some(&transport_meta.0)));

    ToolSessionContext { workspace }
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
