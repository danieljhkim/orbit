use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::{OrbitError, ToolSchema};
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};

use super::OrbitToolServer;
use super::name_map::{ToolNameCollision, build_name_map};
use super::schema::schema_to_tool;
use super::structured::mcp_structured_content;
use crate::error::tool_error_result;

impl OrbitToolServer {
    fn refresh_name_map(&self, schemas: &[ToolSchema]) -> Result<(), ToolNameCollision> {
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

    fn replace_name_map(&self, map: HashMap<String, String>) {
        if let Ok(mut guard) = self.name_map.write() {
            *guard = map;
        }
    }

    fn clear_name_map(&self) {
        if let Ok(mut guard) = self.name_map.write() {
            guard.clear();
        }
    }

    fn canonical_name(&self, advertised: &str) -> Result<String, ToolNameCollision> {
        let schemas = self.host.list_tool_schemas();
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
        let exec_name = canonical.clone();
        let input_for_learning = input.clone();
        let join = tokio::task::spawn_blocking(move || host.call_tool(&exec_name, input)).await;

        match join {
            Ok(Ok(value)) => {
                let value = self
                    .maybe_attach_learning_sidecar(&canonical, input_for_learning, value)
                    .await?;
                Ok(CallToolResult::structured(mcp_structured_content(value)))
            }
            Ok(Err(orbit_err)) => Ok(tool_error_result(&orbit_err)),
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
        let mut schemas = self.host.list_tool_schemas();
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rmcp::model::CallToolRequestParams;
    use serde_json::{Value, json};

    use super::super::OrbitToolServer;
    use super::super::name_map::sanitize_tool_name;
    use super::super::test_support::{EchoArrayHost, StubHost, tool_schema};

    #[test]
    fn refresh_name_map_rejects_listing_collisions() {
        let host = Arc::new(StubHost {
            schemas: Vec::new(),
        });
        let server = OrbitToolServer::new(host);
        let schemas = vec![tool_schema("foo.bar"), tool_schema("foo_bar")];
        let err = server
            .refresh_name_map(&schemas)
            .expect_err("tools/list refresh must reject ambiguous advertised names");
        assert_eq!(err.advertised_name, "foo_bar");
    }

    #[tokio::test]
    async fn call_tool_wraps_affected_array_results_for_strict_mcp_clients() {
        let affected_tools = [
            "orbit.task.list",
            "orbit.task.search",
            "orbit.task.review_thread.list",
        ];
        let host = Arc::new(EchoArrayHost {
            schemas: affected_tools
                .iter()
                .map(|name| tool_schema(name))
                .collect(),
        });
        let server = OrbitToolServer::new(host);

        for canonical_name in affected_tools {
            let result = server
                .call_tool_request(CallToolRequestParams::new(sanitize_tool_name(
                    canonical_name,
                )))
                .await
                .expect("MCP bridge call succeeds");
            let structured = result
                .structured_content
                .as_ref()
                .expect("structured content");

            assert!(
                structured.is_object(),
                "{canonical_name} structuredContent must be object-shaped"
            );
            assert_eq!(
                structured.get("items"),
                Some(&json!([{ "tool": canonical_name }]))
            );

            let wire = serde_json::to_value(&result).expect("serialize CallToolResult");
            assert!(
                wire.get("structuredContent").is_some_and(Value::is_object),
                "{canonical_name} serialized structuredContent must satisfy record validators"
            );
        }
    }

    #[test]
    fn canonical_name_translates_advertised_back_to_dotted() {
        let host = Arc::new(StubHost {
            schemas: vec![tool_schema("orbit.task.add")],
        });
        let server = OrbitToolServer::new(host);
        // Refreshes from host before resolving the advertised name.
        assert_eq!(
            server.canonical_name("orbit_task_add").unwrap(),
            "orbit.task.add"
        );
        // Repeated lookups preserve the same advertised-to-canonical mapping.
        assert_eq!(
            server.canonical_name("orbit_task_add").unwrap(),
            "orbit.task.add"
        );
    }

    #[test]
    fn canonical_name_passes_through_unknown_or_legacy_dotted_names() {
        let host = Arc::new(StubHost {
            schemas: vec![tool_schema("orbit.task.add")],
        });
        let server = OrbitToolServer::new(host);
        // Legacy dotted name from an older client falls through unchanged so
        // the host's own tool-not-found handling still runs.
        assert_eq!(
            server.canonical_name("orbit.task.add").unwrap(),
            "orbit.task.add"
        );
        assert_eq!(
            server.canonical_name("totally.unknown").unwrap(),
            "totally.unknown"
        );
    }

    #[test]
    fn canonical_name_rejects_sanitized_dispatch_collisions() {
        let host = Arc::new(StubHost {
            schemas: vec![tool_schema("foo.bar"), tool_schema("foo_bar")],
        });
        let server = OrbitToolServer::new(host);
        let err = server
            .canonical_name("foo_bar")
            .expect_err("dispatch must reject ambiguous advertised names");
        assert_eq!(err.advertised_name, "foo_bar");
        assert_eq!(
            err.canonical_names,
            vec!["foo.bar".to_string(), "foo_bar".to_string()]
        );
    }
}
