use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{
    McpToolDefinition, OrbitError, SPOKE_REGISTRATION_METHOD_V1, SpokeRegistrationRequestV1,
    SpokeRegistrationResultV1, ToolSchema, ToolSessionContext, audit_execution_id,
    validate_mcp_tool_definitions,
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
use super::schema::schema_to_tool;
use super::structured::mcp_structured_content;
use crate::McpToolExtension;
use crate::error::tool_error_result;

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
    pub(super) fn visible_tool_schemas(&self) -> Result<Vec<ToolSchema>, OrbitError> {
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
            .map(|definition| definition.schema)
            .collect())
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
        transport_meta: &Meta,
    ) -> Result<ToolSessionContext, OrbitError> {
        if self.host.accepts_remote_session_context() {
            let Some(remote) = remote_session_context_from_meta(transport_meta)? else {
                return Err(OrbitError::InvalidInput(
                    "hub tool calls require connector-owned remote session metadata".to_string(),
                ));
            };
            let trusted = self.session_context();
            if remote.transport != Some(orbit_common::types::McpTransport::SshMcp) {
                return Err(OrbitError::InvalidInput(
                    "hub remote session metadata must declare ssh-mcp transport".to_string(),
                ));
            }
            if remote.caller_machine_id.is_none()
                || remote.caller_host_id.is_none()
                || remote.origin_session_id.is_none()
                || remote.mcp_call_id.is_none()
            {
                return Err(OrbitError::InvalidInput(
                    "hub remote session metadata requires caller identity and call correlation"
                        .to_string(),
                ));
            }
            if remote.process_machine_id.is_some() || remote.process_host_id.is_some() {
                return Err(OrbitError::InvalidInput(
                    "hub remote session metadata may not claim process identity".to_string(),
                ));
            }
            let mut remote = remote;
            // The fixed server capability is authority; the connector cannot
            // expand it through per-call metadata.
            remote.effective_capabilities = trusted.effective_capabilities;
            // Lease correlation is a trusted runner/broker seam. A spoke may
            // not attach an arbitrary run or lease to a hub audit record.
            remote.leased_run = None;
            return Ok(remote);
        }
        let mut context = self.session_context();
        context.mcp_call_id = Some(audit_execution_id("mcall"));
        Ok(context)
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
        let session_context = match self.session_context_for_call(transport_meta) {
            Ok(context) => context,
            Err(denial) => {
                let context = self.session_context();
                let denial =
                    self.host
                        .reject_tool_call(req.name.as_ref(), &Value::Null, &context, denial);
                return Ok(tool_error_result(&denial));
            }
        };
        let inbound = req.name.to_string();
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
            Ok(Ok(value)) => {
                let value = self
                    .maybe_attach_learning_sidecar(&canonical, input_for_learning, value)
                    .await?;
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
        let instructions = self.host.private_server_instructions().unwrap_or_else(|| {
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
        let mut schemas = self
            .visible_tool_schemas()
            .map_err(invalid_definitions_mcp_error)?;
        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        self.refresh_name_map(&schemas)
            .map_err(ToolNameCollision::into_mcp_error)?;
        let tools = schemas.into_iter().map(schema_to_tool).collect();
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
        if method != SPOKE_REGISTRATION_METHOD_V1 || !self.host.accepts_remote_session_context() {
            return Err(McpError::new(ErrorCode::METHOD_NOT_FOUND, method, None));
        }
        if remote_session_context_from_meta(&ctx.meta)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?
            .is_none()
        {
            return Err(McpError::invalid_params(
                "private spoke registration requires connector-owned remote session metadata",
                None,
            ));
        }
        let session_context = self
            .session_context_for_call(&ctx.meta)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let registration = request
            .params_as::<SpokeRegistrationRequestV1>()
            .map_err(|error| {
                McpError::invalid_params(
                    format!("invalid private spoke registration payload: {error}"),
                    None,
                )
            })?
            .ok_or_else(|| {
                McpError::invalid_params("private spoke registration requires parameters", None)
            })?;
        if let Err(error) = registration.validate() {
            return registration_custom_result(SpokeRegistrationResultV1::rejected(&error));
        }

        let host = Arc::clone(&self.host);
        let outcome = tokio::task::spawn_blocking(move || {
            host.private_register_spoke(registration, session_context)
        })
        .await
        .map_err(|error| {
            McpError::internal_error(
                format!("private spoke registration worker failed: {error}"),
                None,
            )
        })?;
        let Some(outcome) = outcome else {
            return Err(McpError::new(ErrorCode::METHOD_NOT_FOUND, method, None));
        };
        let result = match outcome {
            Ok(result) => result,
            Err(error) => SpokeRegistrationResultV1::rejected(&error),
        };
        result.validate().map_err(|error| {
            McpError::internal_error(
                format!("invalid private spoke registration result: {error}"),
                None,
            )
        })?;
        registration_custom_result(result)
    }
}

fn registration_custom_result(result: SpokeRegistrationResultV1) -> Result<CustomResult, McpError> {
    serde_json::to_value(result)
        .map(CustomResult::new)
        .map_err(|error| {
            McpError::internal_error(
                format!("serialize private spoke registration result: {error}"),
                None,
            )
        })
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

fn remote_session_context_from_meta(meta: &Meta) -> Result<Option<ToolSessionContext>, OrbitError> {
    let Some(value) = meta
        .0
        .get("orbit")
        .and_then(|orbit| orbit.get(crate::client::REMOTE_SESSION_META_KEY))
    else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| {
            OrbitError::InvalidInput(format!("invalid hub remote session metadata: {error}"))
        })
}
