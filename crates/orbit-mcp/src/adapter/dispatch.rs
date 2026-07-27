use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{
    McpToolDefinition, OrbitError, ToolSchema, ToolSessionContext, validate_mcp_tool_definitions,
};
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, CustomRequest, CustomResult, ErrorCode, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, Meta, PaginatedRequestParams,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};

use super::OrbitToolServer;
use super::name_map::{ToolNameCollision, build_name_map};
use super::schema::{ensure_workspace_selector, schema_to_tool};
use super::structured::mcp_structured_content;
use crate::error::tool_error_result;
use crate::{McpCustomRequestError, McpRequestKind, McpResultDecoration, McpToolExtension};

impl OrbitToolServer {
    pub(super) fn combined_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let mut definitions = self.host.list_mcp_tool_definitions()?;
        definitions.retain(|definition| {
            !self
                .extensions
                .iter()
                .any(|registration| registration.extension().recognizes(&definition.schema.name))
        });
        for registration in self
            .extensions
            .iter()
            .filter(|registration| registration.advertises_definitions())
        {
            let extension_definitions = registration.extension().definitions()?;
            for definition in &extension_definitions {
                if !registration.extension().recognizes(&definition.schema.name) {
                    return Err(OrbitError::InvalidInput(format!(
                        "in-process MCP extension definition '{}' is not recognized by its owner",
                        definition.schema.name
                    )));
                }
                let _owner = self.extension_for(&definition.schema.name)?;
            }
            definitions.extend(extension_definitions);
        }
        validate_mcp_tool_definitions(&definitions)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        Ok(definitions)
    }

    fn extension_for(&self, name: &str) -> Result<Option<Arc<dyn McpToolExtension>>, OrbitError> {
        let mut matches = self
            .extensions
            .iter()
            .filter(|registration| registration.extension().recognizes(name));
        let Some(first) = matches.next() else {
            return Ok(None);
        };
        if matches.next().is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "multiple in-process MCP extensions recognize tool '{name}'"
            )));
        }
        Ok(Some(Arc::clone(first.extension())))
    }

    #[cfg(test)]
    pub(super) fn combined_tool_schemas(&self) -> Result<Vec<ToolSchema>, OrbitError> {
        Ok(self
            .combined_tool_definitions()?
            .into_iter()
            .map(|definition| definition.schema)
            .collect())
    }

    /// Return the advertised subset for the trusted effective session grants.
    ///
    /// Capability sets are deliberately non-hierarchical. An empty set is a
    /// hard denial, and a definition is visible only when at least one of its
    /// adjacent allowed capabilities is present in the session set. Canonical
    /// name resolution still uses the unfiltered registry so a call to a
    /// hidden tool reaches the host's audited denial path instead of being
    /// misclassified as an unknown name.
    #[cfg(test)]
    pub(super) fn visible_tool_schemas(&self) -> Result<Vec<ToolSchema>, OrbitError> {
        Ok(self
            .visible_tool_definitions()?
            .into_iter()
            .map(|definition| definition.schema)
            .collect())
    }

    pub(super) fn visible_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let context = self.session_context();
        Ok(self
            .combined_tool_definitions()?
            .into_iter()
            .filter(|definition| {
                definition
                    .policy
                    .allowed_capabilities()
                    .iter()
                    .any(|capability| context.effective_capabilities.contains(capability))
            })
            .collect())
    }

    /// Resolve the advertised wire input schema for one canonical definition.
    ///
    /// Whoever owns the schema — the host resolver or an in-process extension —
    /// the broker's workspace routing contract is the adapter's to advertise,
    /// so the selector is injected here rather than duplicated in every
    /// resolver.
    pub(super) fn input_schema_for(
        &self,
        definition: &McpToolDefinition,
    ) -> Result<Map<String, Value>, OrbitError> {
        let mut schema = if let Some(extension) = self.extension_for(&definition.schema.name)? {
            extension.input_schema(definition)?
        } else {
            self.input_schema_resolver.input_schema(definition)?
        };
        ensure_workspace_selector(&mut schema, definition);
        Ok(schema)
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

    fn session_context_for_call(
        &self,
        request: &McpRequestKind,
        transport_meta: &Meta,
    ) -> Result<ToolSessionContext, OrbitError> {
        self.call_context_resolver
            .resolve(&self.session_context(), request, &transport_meta.0)
    }

    pub(super) fn canonical_name(&self, advertised: &str) -> Result<String, McpError> {
        let schemas = self
            .combined_tool_definitions()
            .map(|definitions| {
                definitions
                    .into_iter()
                    .map(|definition| definition.schema)
                    .collect::<Vec<_>>()
            })
            .map_err(invalid_definitions_mcp_error)?;
        let map = match build_name_map(&schemas) {
            Ok(map) => map,
            Err(err) => {
                self.clear_name_map();
                return Err(err.into_mcp_error());
            }
        };
        let resolved = map.get(advertised).cloned();
        self.replace_name_map(map);
        Ok(resolved.unwrap_or_else(|| advertised.to_string()))
    }

    #[cfg(test)]
    pub(super) async fn call_tool_request(
        &self,
        req: CallToolRequestParams,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool_request_with_meta(req, &Meta::default())
            .await
    }

    async fn call_tool_request_with_meta(
        &self,
        req: CallToolRequestParams,
        transport_meta: &Meta,
    ) -> Result<CallToolResult, McpError> {
        // Generate exactly once before name/exposure preflight. Every dispatch
        // and denial path below receives this same trusted call context.
        let inbound = req.name.to_string();
        let request_kind = McpRequestKind::Tool {
            name: inbound.clone(),
        };
        let session_context = match self.session_context_for_call(&request_kind, transport_meta) {
            Ok(context) => context,
            Err(denial) => {
                let context = self.session_context();
                let denial =
                    self.host
                        .reject_tool_call(req.name.as_ref(), &Value::Null, &context, denial);
                return Ok(tool_error_result(&denial));
            }
        };
        if let Err(denial) = self.host.preflight_tool_call(&inbound, &session_context) {
            let denial =
                self.host
                    .reject_tool_call(&inbound, &Value::Null, &session_context, denial);
            return Ok(tool_error_result(&denial));
        }
        let canonical = self.canonical_name(&inbound)?;
        let input = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let definition = self
            .combined_tool_definitions()
            .map_err(invalid_definitions_mcp_error)?
            .into_iter()
            .find(|definition| definition.schema.name == canonical);
        if let Some(definition) = definition
            && !definition
                .policy
                .allowed_capabilities()
                .iter()
                .any(|capability| session_context.effective_capabilities.contains(capability))
        {
            let allowed = definition
                .policy
                .allowed_capabilities()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let denial = OrbitError::InvalidInput(format!(
                "MCP capability denied for tool '{canonical}': the effective session set must contain one of [{allowed}]"
            ));
            let denial = self
                .host
                .reject_tool_call(&canonical, &input, &session_context, denial);
            return Ok(tool_error_result(&denial));
        }

        let extension = self
            .extension_for(&canonical)
            .map_err(invalid_definitions_mcp_error)?;
        let extension_for_dispatch = extension.clone();
        let host = Arc::clone(&self.host);
        let exec_name = canonical.clone();
        let input_for_learning = input.clone();
        let call_context = session_context.clone();
        let server_context = self.session_context();
        // Extension recognition is deliberately independent of advertised
        // schemas. Re-exposing a host schema must not make an in-process call
        // bypass the host's policy/audit seam.
        let join = tokio::task::spawn_blocking(move || {
            if let Some(extension) = extension_for_dispatch {
                let extension_name = exec_name.clone();
                let mut dispatch = move |input, session_context| {
                    extension.call(&extension_name, input, session_context)
                };
                host.call_in_process_tool(&exec_name, input, session_context, &mut dispatch)
            } else {
                host.call_tool(&exec_name, input, session_context)
            }
        })
        .await;

        match join {
            Ok(Ok(mut value)) => {
                for decorator in &self.result_decorators {
                    value = decorator
                        .decorate(McpResultDecoration {
                            canonical_name: canonical.clone(),
                            input: input_for_learning.clone(),
                            output: value,
                            call_context: call_context.clone(),
                            server_context: server_context.clone(),
                        })
                        .await
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                }
                Ok(CallToolResult::structured(mcp_structured_content(value)))
            }
            Ok(Err(orbit_err)) => {
                if let Some(extension) = extension.as_ref() {
                    extension.report_call_failure(&canonical, &orbit_err);
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
        let announced = session_context_from_initialize(&request, &context.meta);
        let mut trusted = self.session_context();
        // The external initialize request controls only the legacy address
        // selector. Identity, transport, grants, and correlation remain the
        // adapter-owned values installed at construction.
        trusted.workspace = announced.workspace;
        trusted.mcp_call_id = None;
        self.replace_session_context(trusted);
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        std::future::ready(Ok(self.get_info()))
    }

    fn get_info(&self) -> ServerInfo {
        let implementation = Implementation::new("orbit-mcp", env!("CARGO_PKG_VERSION"));
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let instructions = self
            .metadata
            .instructions()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                "Orbit tool registry exposed over MCP. Call tools/list to discover available \
                 task, graph, state, and review operations; each tool advertises its own input \
                 schema."
                    .to_string()
            });
        InitializeResult::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(instructions)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut definitions = self
            .visible_tool_definitions()
            .map_err(invalid_definitions_mcp_error)?;
        definitions.sort_by(|a, b| a.schema.name.cmp(&b.schema.name));
        let schemas = definitions
            .iter()
            .map(|definition| definition.schema.clone())
            .collect::<Vec<_>>();
        self.refresh_name_map(&schemas)
            .map_err(ToolNameCollision::into_mcp_error)?;
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
        req: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool_request_with_meta(req, &ctx.meta).await
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CustomResult, McpError> {
        let method = request.method.clone();
        let mut handlers = self
            .custom_request_handlers
            .iter()
            .filter(|handler| handler.recognizes(&method));
        let Some(handler) = handlers.next().cloned() else {
            return Err(McpError::new(ErrorCode::METHOD_NOT_FOUND, method, None));
        };
        if handlers.next().is_some() {
            return Err(McpError::internal_error(
                format!("multiple MCP custom request handlers recognize method '{method}'"),
                None,
            ));
        }
        let request_kind = McpRequestKind::Custom {
            method: method.clone(),
        };
        let session_context = self
            .session_context_for_call(&request_kind, &ctx.meta)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let worker_label = handler.worker_label();
        let method_for_handler = method.clone();
        let result = tokio::task::spawn_blocking(move || {
            handler.call(&method_for_handler, request.params, session_context)
        })
        .await
        .map_err(|error| {
            McpError::internal_error(format!("{worker_label} worker failed: {error}"), None)
        })?
        .map_err(|error| custom_request_error_to_mcp(error, &method))?;
        Ok(CustomResult::new(result))
    }
}

fn custom_request_error_to_mcp(error: McpCustomRequestError, method: &str) -> McpError {
    match error {
        McpCustomRequestError::MethodNotFound => {
            McpError::new(ErrorCode::METHOD_NOT_FOUND, method.to_string(), None)
        }
        McpCustomRequestError::InvalidParams { message, data } => {
            McpError::invalid_params(message, data)
        }
        McpCustomRequestError::Internal { message, data } => {
            McpError::internal_error(message, data)
        }
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
    transport_meta: &rmcp::model::Meta,
) -> ToolSessionContext {
    // Clients deliberately announce workspace through initialize `_meta`.
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
