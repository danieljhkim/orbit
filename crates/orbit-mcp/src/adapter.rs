use std::collections::{BTreeMap, HashMap};
use std::fmt;
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
/// `name_map` is rebuilt from the host on every `tools/list` and
/// `tools/call` so dynamically-added tools cannot create stale or
/// ambiguous dispatch.
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

    async fn call_tool_request(
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
        let join = tokio::task::spawn_blocking(move || host.call_tool(&exec_name, input)).await;

        match join {
            Ok(Ok(value)) => Ok(CallToolResult::structured(mcp_structured_content(value))),
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

/// Sanitize an Orbit tool name into the character set MCP clients accept.
///
/// Cursor enforces `[a-zA-Z0-9_]` and VS Code enforces `[a-z0-9_-]`. Replacing
/// `.` with `_` keeps Orbit's existing names within the intersection of both
/// rule sets without renaming any internal canonical identifier.
fn sanitize_tool_name(name: &str) -> String {
    name.replace('.', "_")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolNameCollision {
    advertised_name: String,
    canonical_names: Vec<String>,
}

impl ToolNameCollision {
    fn into_mcp_error(self) -> McpError {
        let message = self.to_string();
        McpError::internal_error(
            message,
            Some(json!({
                "code": "tool_name_collision",
                "advertised_name": self.advertised_name,
                "canonical_names": self.canonical_names,
            })),
        )
    }
}

impl fmt::Display for ToolNameCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MCP tool name collision: advertised name '{}' is produced by canonical tools {}; rename one tool before exposing over MCP",
            self.advertised_name,
            self.canonical_names.join(", ")
        )
    }
}

fn build_name_map(schemas: &[ToolSchema]) -> Result<HashMap<String, String>, ToolNameCollision> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for schema in schemas {
        grouped
            .entry(sanitize_tool_name(&schema.name))
            .or_default()
            .push(schema.name.clone());
    }

    let mut map = HashMap::with_capacity(schemas.len());
    for (advertised_name, mut canonical_names) in grouped {
        canonical_names.sort();
        canonical_names.dedup();
        if canonical_names.len() > 1 {
            return Err(ToolNameCollision {
                advertised_name,
                canonical_names,
            });
        }
        if let Some(canonical_name) = canonical_names.pop() {
            map.insert(advertised_name, canonical_name);
        }
    }
    Ok(map)
}

/// Keep MCP `structuredContent` object-shaped for clients that enforce record
/// results (notably Cursor and VS Code), while preserving non-object payloads.
fn mcp_structured_content(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        Value::Array(items) => json!({ "items": items }),
        value => json!({ "value": value }),
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
    use serde_json::json;

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

    fn tool_schema(name: &str) -> ToolSchema {
        ToolSchema {
            name: name.to_string(),
            description: String::new(),
            parameters: Vec::new(),
            builtin: true,
        }
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
            tool_schema("orbit.task.add"),
            tool_schema("orbit.task.review_thread.add"),
        ];
        let map = build_name_map(&schemas).expect("unique advertised names");
        assert_eq!(
            map.get("orbit_task_add").map(String::as_str),
            Some("orbit.task.add")
        );
        assert_eq!(
            map.get("orbit_task_review_thread_add").map(String::as_str),
            Some("orbit.task.review_thread.add")
        );
    }

    #[test]
    fn build_name_map_rejects_sanitized_name_collisions() {
        let schemas = vec![tool_schema("foo.bar"), tool_schema("foo_bar")];
        let err = build_name_map(&schemas).expect_err("sanitized names must be unique");
        assert_eq!(err.advertised_name, "foo_bar");
        assert_eq!(
            err.canonical_names,
            vec!["foo.bar".to_string(), "foo_bar".to_string()]
        );

        let mcp_err = err.into_mcp_error();
        assert!(mcp_err.message.contains("foo_bar"));
        let data = mcp_err.data.as_ref().expect("structured error data");
        assert_eq!(
            data.get("code").and_then(Value::as_str),
            Some("tool_name_collision")
        );
        assert_eq!(
            data.get("advertised_name").and_then(Value::as_str),
            Some("foo_bar")
        );
    }

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

    #[test]
    fn schema_to_tool_keeps_dotted_orbit_tools_advertised_with_underscores() {
        let tool = schema_to_tool(tool_schema("orbit.task.add"));
        assert_eq!(tool.name.as_ref(), "orbit_task_add");
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
    fn mcp_structured_content_preserves_existing_objects() {
        let value = json!({ "ok": true });
        assert_eq!(mcp_structured_content(value.clone()), value);
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

    struct EchoArrayHost {
        schemas: Vec<ToolSchema>,
    }

    impl crate::McpHost for EchoArrayHost {
        fn list_tool_schemas(&self) -> Vec<ToolSchema> {
            self.schemas.clone()
        }

        fn call_tool(&self, name: &str, _input: Value) -> Result<Value, OrbitError> {
            Ok(json!([{ "tool": name }]))
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
        // the host's own ToolNotFound handling still runs.
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
