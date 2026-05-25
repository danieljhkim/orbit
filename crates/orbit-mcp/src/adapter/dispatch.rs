use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use orbit_common::types::{OrbitError, ToolSchema, ToolSessionContext};
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
use crate::{McpToolAudit, McpToolAuditStatus};

impl OrbitToolServer {
    pub(super) fn combined_tool_schemas(&self) -> Vec<ToolSchema> {
        let mut schemas: Vec<_> = self
            .host
            .list_tool_schemas()
            .into_iter()
            .filter(|schema| !self.graph_tools.is_graph_tool(&schema.name))
            .collect();
        schemas.extend(self.graph_tools.schemas());
        schemas
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
        let input = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let host = Arc::clone(&self.host);
        let graph_tools = Arc::clone(&self.graph_tools);
        let exec_name = canonical.clone();
        let session_context = self.session_context();
        let input_for_learning = input.clone();
        let graph_tool = self.graph_tools.is_graph_tool(&canonical);
        let graph_backend = if graph_tool {
            match self.graph_tools.resolve_backend(&input) {
                Ok(backend) => Some(backend),
                Err(err) => {
                    record_direct_graph_audit(
                        host.as_ref(),
                        &canonical,
                        input.clone(),
                        None,
                        McpToolAuditStatus::Failure,
                        Some(err.to_string()),
                        1,
                    );
                    return Ok(tool_error_result(&err));
                }
            }
        } else {
            None
        };
        let direct_audit_backend = graph_backend
            .filter(|backend| !self.graph_tools.host_audits_call(&canonical, *backend))
            .map(|backend| backend.as_str().to_string());
        let start = Instant::now();
        let input_for_audit = input.clone();
        let join = tokio::task::spawn_blocking(move || {
            if graph_tool {
                let legacy_host = Arc::clone(&host);
                let shadow_host = Arc::clone(&host);
                graph_tools.call_tool(
                    &exec_name,
                    input,
                    session_context,
                    graph_backend.expect("graph backend resolved before graph dispatch"),
                    |name, input, context| legacy_host.call_tool(name, input, context),
                    |name, input, context| shadow_host.call_shadow_tool(name, input, context),
                )
            } else {
                host.call_tool(&exec_name, input, session_context)
            }
        })
        .await;

        match join {
            Ok(Ok(value)) => {
                if let Some(backend) = direct_audit_backend.as_deref() {
                    record_direct_graph_audit(
                        self.host.as_ref(),
                        &canonical,
                        input_for_audit.clone(),
                        Some(backend.to_string()),
                        McpToolAuditStatus::Success,
                        None,
                        elapsed_ms(start),
                    );
                }
                let value = self
                    .maybe_attach_learning_sidecar(&canonical, input_for_learning, value)
                    .await?;
                Ok(CallToolResult::structured(mcp_structured_content(value)))
            }
            Ok(Err(orbit_err)) => {
                if let Some(backend) = direct_audit_backend.as_deref() {
                    record_direct_graph_audit(
                        self.host.as_ref(),
                        &canonical,
                        input_for_audit.clone(),
                        Some(backend.to_string()),
                        McpToolAuditStatus::Failure,
                        Some(orbit_err.to_string()),
                        elapsed_ms(start),
                    );
                }
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
                if let Some(backend) = direct_audit_backend.as_deref() {
                    record_direct_graph_audit(
                        self.host.as_ref(),
                        &canonical,
                        input_for_audit,
                        Some(backend.to_string()),
                        McpToolAuditStatus::Failure,
                        Some(err.to_string()),
                        elapsed_ms(start),
                    );
                }
                Ok(tool_error_result(&err))
            }
        }
    }
}

fn elapsed_ms(start: Instant) -> i64 {
    (start.elapsed().as_millis() as i64).max(1)
}

fn record_direct_graph_audit(
    host: &dyn crate::McpHost,
    name: &str,
    input: Value,
    backend: Option<String>,
    status: McpToolAuditStatus,
    error_message: Option<String>,
    duration_ms: i64,
) {
    if let Err(err) = host.record_tool_audit(McpToolAudit {
        name: name.to_string(),
        input,
        status,
        duration_ms,
        error_message,
        backend,
    }) {
        tracing::warn!(
            target: "orbit.mcp.audit",
            tool = %name,
            error = %err,
            "failed to persist direct MCP graph audit event"
        );
    }
}

impl ServerHandler for OrbitToolServer {
    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        self.replace_session_context(session_context_from_initialize(&request));
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
) -> ToolSessionContext {
    // ADR-0181: clients deliberately announce workspace through initialize `_meta`.
    let workspace = request
        .meta
        .as_ref()
        .and_then(|meta| {
            meta.0
                .get("orbit")
                .and_then(|orbit| orbit.get("workspace"))
                .or_else(|| meta.0.get("orbit.workspace"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    ToolSessionContext { workspace }
}
