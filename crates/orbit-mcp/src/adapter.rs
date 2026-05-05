use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
///
/// Orbit's canonical tool names use dots (`orbit.task.add`), but several MCP
/// clients (Cursor, VS Code) reject names containing characters outside
/// `[a-z0-9_-]` and refuse to load the tool. The adapter sanitizes names by
/// replacing dots with underscores when advertising over MCP and translates
/// inbound `tools/call` names back to canonical form before dispatch. The
/// `name_map` is rebuilt from the host on every `tools/list` and refreshed
/// lazily on a `tools/call` cache miss so legacy dotted names from older
/// clients still dispatch correctly.
pub struct OrbitToolServer {
    host: Arc<dyn McpHost>,
    name_map: RwLock<HashMap<String, String>>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        Self {
            host,
            name_map: RwLock::new(HashMap::new()),
        }
    }

    fn refresh_name_map(&self, schemas: &[ToolSchema]) {
        let map = build_name_map(schemas);
        if let Ok(mut guard) = self.name_map.write() {
            *guard = map;
        }
    }

    fn canonical_name(&self, advertised: &str) -> String {
        if let Ok(guard) = self.name_map.read()
            && let Some(name) = guard.get(advertised)
        {
            return name.clone();
        }
        let schemas = self.host.list_tool_schemas();
        let map = build_name_map(&schemas);
        let resolved = map.get(advertised).cloned();
        if let Ok(mut guard) = self.name_map.write() {
            *guard = map;
        }
        resolved.unwrap_or_else(|| advertised.to_string())
    }
}

/// Sanitize an Orbit tool name into the character set MCP clients accept.
///
/// Cursor enforces `[a-zA-Z0-9_]` and VS Code enforces `[a-z0-9_-]`. Replacing
/// `.` with `_` keeps Orbit's existing names within the intersection of both
/// rule sets without renaming any internal canonical identifier.
fn sanitize_tool_name(name: &str) -> String {
    name.replace('.', "_")
}

fn build_name_map(schemas: &[ToolSchema]) -> HashMap<String, String> {
    let mut map = HashMap::with_capacity(schemas.len());
    for schema in schemas {
        let advertised = sanitize_tool_name(&schema.name);
        map.insert(advertised, schema.name.clone());
    }
    map
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
        self.refresh_name_map(&schemas);
        let tools = schemas.into_iter().map(schema_to_tool).collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let inbound = req.name.to_string();
        let canonical = self.canonical_name(&inbound);
        let input = req
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let host = Arc::clone(&self.host);
        let exec_name = canonical.clone();
        let join = tokio::task::spawn_blocking(move || host.call_tool(&exec_name, input)).await;

        match join {
            Ok(Ok(value)) => Ok(CallToolResult::structured(value)),
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

fn schema_to_tool(schema: ToolSchema) -> Tool {
    let description = schema.description.clone();
    let input_schema = build_input_schema(&schema.name, &schema.parameters);
    let advertised_name = sanitize_tool_name(&schema.name);
    Tool::new(advertised_name, description, Arc::new(input_schema))
}

fn build_input_schema(tool_name: &str, params: &[ToolParam]) -> JsonObject {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for param in params {
        let mut prop = property_for(&param.param_type);
        if let Some(values) = enum_values_for(tool_name, &param.name) {
            prop.insert(
                "enum".to_string(),
                Value::Array(
                    values
                        .iter()
                        .map(|value| Value::String((*value).to_string()))
                        .collect(),
                ),
            );
        }
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

const TASK_TYPE_ENUM: &[&str] = &[
    "task", "feature", "epic", "friction", "issue", "bug", "chore", "refactor",
];

const TASK_STATUS_ENUM: &[&str] = &[
    "proposed",
    "friction",
    "backlog",
    "someday",
    "in-progress",
    "review",
    "done",
    "blocked",
    "rejected",
];

fn enum_values_for(tool_name: &str, param_name: &str) -> Option<&'static [&'static str]> {
    match (tool_name, param_name) {
        ("orbit.task.add", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.add" | "orbit.task.update", "status") => Some(TASK_STATUS_ENUM),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn param_with_type(name: &str, param_type: &str) -> ToolParam {
        ToolParam {
            name: name.to_string(),
            description: String::new(),
            param_type: param_type.to_string(),
            required: false,
        }
    }

    fn param(name: &str) -> ToolParam {
        param_with_type(name, "string")
    }

    #[test]
    fn task_add_schema_advertises_type_and_status_enums() {
        let schema = build_input_schema("orbit.task.add", &[param("type"), param("status")]);
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties");

        let type_enum = properties["type"]["enum"].as_array().expect("type enum");
        assert!(type_enum.iter().any(|value| value == "friction"));

        let status_enum = properties["status"]["enum"]
            .as_array()
            .expect("status enum");
        assert!(status_enum.iter().any(|value| value == "friction"));
    }

    #[test]
    fn task_update_schema_advertises_friction_status_enum() {
        let schema = build_input_schema("orbit.task.update", &[param("status")]);
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties");
        let status_enum = properties["status"]["enum"]
            .as_array()
            .expect("status enum");
        assert!(status_enum.iter().any(|value| value == "friction"));
    }

    #[test]
    fn sanitize_tool_name_replaces_dots_with_underscores() {
        assert_eq!(sanitize_tool_name("orbit.task.add"), "orbit_task_add");
        assert_eq!(
            sanitize_tool_name("orbit.task.review_thread.add"),
            "orbit_task_review_thread_add"
        );
        assert_eq!(sanitize_tool_name("orbit_task_add"), "orbit_task_add");
    }

    #[test]
    fn build_name_map_keys_are_advertised_names() {
        let schemas = vec![
            ToolSchema {
                name: "orbit.task.add".to_string(),
                description: String::new(),
                parameters: Vec::new(),
                builtin: true,
            },
            ToolSchema {
                name: "orbit.task.review_thread.add".to_string(),
                description: String::new(),
                parameters: Vec::new(),
                builtin: true,
            },
        ];
        let map = build_name_map(&schemas);
        assert_eq!(
            map.get("orbit_task_add").map(String::as_str),
            Some("orbit.task.add")
        );
        assert_eq!(
            map.get("orbit_task_review_thread_add").map(String::as_str),
            Some("orbit.task.review_thread.add")
        );
    }

    struct StubHost {
        schemas: Vec<ToolSchema>,
    }

    impl crate::McpHost for StubHost {
        fn list_tool_schemas(&self) -> Vec<ToolSchema> {
            self.schemas.clone()
        }

        fn call_tool(&self, _name: &str, _input: Value) -> Result<Value, OrbitError> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn canonical_name_translates_advertised_back_to_dotted() {
        let host = Arc::new(StubHost {
            schemas: vec![ToolSchema {
                name: "orbit.task.add".to_string(),
                description: String::new(),
                parameters: Vec::new(),
                builtin: true,
            }],
        });
        let server = OrbitToolServer::new(host);
        // Cache miss path: refreshes from host on first lookup.
        assert_eq!(server.canonical_name("orbit_task_add"), "orbit.task.add");
        // Cache hit path: same answer, served from in-memory map.
        assert_eq!(server.canonical_name("orbit_task_add"), "orbit.task.add");
    }

    #[test]
    fn canonical_name_passes_through_unknown_or_legacy_dotted_names() {
        let host = Arc::new(StubHost {
            schemas: vec![ToolSchema {
                name: "orbit.task.add".to_string(),
                description: String::new(),
                parameters: Vec::new(),
                builtin: true,
            }],
        });
        let server = OrbitToolServer::new(host);
        // Legacy dotted name from an older client falls through unchanged so
        // the host's own ToolNotFound handling still runs.
        assert_eq!(server.canonical_name("orbit.task.add"), "orbit.task.add");
        assert_eq!(server.canonical_name("totally.unknown"), "totally.unknown");
    }

    #[test]
    fn task_dependency_schemas_accept_string_or_string_array() {
        for tool_name in ["orbit.task.add", "orbit.task.update"] {
            let schema =
                build_input_schema(tool_name, &[param_with_type("dependencies", "string_list")]);
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .expect("properties");
            let dependencies = properties
                .get("dependencies")
                .and_then(Value::as_object)
                .expect("dependencies property");
            let any_of = dependencies
                .get("anyOf")
                .and_then(Value::as_array)
                .expect("string-list union");

            assert!(
                any_of.iter().any(|schema| {
                    schema.get("type").and_then(Value::as_str) == Some("array")
                        && schema
                            .get("items")
                            .and_then(|items| items.get("type"))
                            .and_then(Value::as_str)
                            == Some("string")
                }),
                "{tool_name} dependencies must accept an array of strings"
            );
            assert!(
                any_of
                    .iter()
                    .any(|schema| schema.get("type").and_then(Value::as_str) == Some("string")),
                "{tool_name} dependencies must accept a string"
            );
        }
    }
}
