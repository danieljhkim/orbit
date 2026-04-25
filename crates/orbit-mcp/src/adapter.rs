use std::sync::Arc;

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

use crate::McpHost;
use crate::error::tool_error_result;

/// An rmcp [`ServerHandler`] that delegates tool listing and tool execution to
/// an injected [`McpHost`].
///
/// Tools are enumerated on every `tools/list` request so late-registered or
/// newly-enabled tools become visible without a restart. Each `tools/call`
/// fans into the host's synchronous executor via [`tokio::task::spawn_blocking`]
/// because Orbit tool implementations issue blocking filesystem, git, and
/// SQLite calls.
pub struct OrbitToolServer {
    host: Arc<dyn McpHost>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        Self { host }
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
        let tools = schemas.into_iter().map(schema_to_tool).collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = req.name.to_string();
        let input = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let host = Arc::clone(&self.host);
        let exec_name = name.clone();
        let join = tokio::task::spawn_blocking(move || host.call_tool(&exec_name, input)).await;

        match join {
            Ok(Ok(value)) => Ok(CallToolResult::structured(value)),
            Ok(Err(orbit_err)) => Ok(tool_error_result(&orbit_err)),
            Err(join_err) => {
                let err = OrbitError::Execution(format!(
                    "tool '{name}' worker panicked or was cancelled: {join_err}"
                ));
                Ok(tool_error_result(&err))
            }
        }
    }
}

fn schema_to_tool(schema: ToolSchema) -> Tool {
    let description = schema.description.clone();
    let input_schema = build_input_schema(&schema.parameters);
    Tool::new(schema.name, description, Arc::new(input_schema))
}

fn build_input_schema(params: &[ToolParam]) -> JsonObject {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for param in params {
        let mut prop = property_for(&param.param_type);
        if !param.description.is_empty() {
            prop.insert(
                "description".to_string(),
                Value::String(param.description.clone()),
            );
        }
        properties.insert(param.name.clone(), Value::Object(prop));

        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }

    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_string(), Value::Array(required));
    }
    // Orbit tools accept identity aliases (`agent`, `model`) and other
    // convenience kwargs not enumerated in their static param list. Permit
    // extra properties so MCP clients aren't blocked by a client-side
    // schema validator.
    schema.insert("additionalProperties".to_string(), Value::Bool(true));
    schema
}

/// Build the JSON-Schema fragment for a single parameter.
///
/// String-list and object-map parameters are emitted as `anyOf` unions because
/// Orbit tool input handlers normalize those specific shapes. Generic arrays
/// stay arrays so arrays of objects are not advertised as string lists.
fn property_for(param_type: &str) -> Map<String, Value> {
    let mut m = Map::new();
    match param_type.trim().to_ascii_lowercase().as_str() {
        "string" | "text" | "enum" => {
            m.insert("type".to_string(), Value::String("string".to_string()));
        }
        "integer" | "int" => {
            m.insert("type".to_string(), Value::String("integer".to_string()));
        }
        "number" | "float" => {
            m.insert("type".to_string(), Value::String("number".to_string()));
        }
        "boolean" | "bool" => {
            m.insert("type".to_string(), Value::String("boolean".to_string()));
        }
        "string_list" | "string[]" | "strings" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "array", "items": { "type": "string" } },
                    { "type": "string" },
                ]),
            );
        }
        "array" | "list" => {
            m.insert("type".to_string(), Value::String("array".to_string()));
        }
        "object" | "map" | "json" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "object" },
                    { "type": "array", "items": { "type": "object" } },
                ]),
            );
        }
        _ => {
            m.insert("type".to_string(), Value::String("string".to_string()));
        }
    }
    m
}
