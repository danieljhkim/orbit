use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};

use orbit_common::types::{
    LearningInjectionCaps, LearningInjectionState, LearningReminder, OrbitError, ToolParam,
    ToolSchema,
};
use orbit_common::utility::learning_session::{
    learning_session_state_path, read_learning_session_state, update_learning_session_state,
};
use orbit_common::utility::selector::anchor_path;
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
    learning_session_id: Option<String>,
    learning_caps: LearningInjectionCaps,
    learning_states: tokio::sync::Mutex<HashMap<String, LearningInjectionState>>,
}

impl OrbitToolServer {
    pub fn new(host: Arc<dyn McpHost>) -> Self {
        let learning_session_id = std::env::var("ORBIT_SESSION_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let learning_caps = LearningInjectionCaps::from_env();
        let mut learning_states = HashMap::new();
        let key = learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string());
        let state = learning_session_id
            .as_deref()
            .and_then(load_learning_state_for_session)
            .unwrap_or_default();
        learning_states.insert(key, state);
        Self {
            host,
            name_map: RwLock::new(HashMap::new()),
            learning_session_id,
            learning_caps,
            learning_states: tokio::sync::Mutex::new(learning_states),
        }
    }

    #[cfg(test)]
    fn new_for_test(
        host: Arc<dyn McpHost>,
        learning_session_id: Option<String>,
        learning_caps: LearningInjectionCaps,
        initial_state: LearningInjectionState,
    ) -> Self {
        let key = learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string());
        let mut learning_states = HashMap::new();
        learning_states.insert(key, initial_state);
        Self {
            host,
            name_map: RwLock::new(HashMap::new()),
            learning_session_id,
            learning_caps,
            learning_states: tokio::sync::Mutex::new(learning_states),
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

    async fn maybe_attach_learning_sidecar(
        &self,
        canonical: &str,
        input: Value,
        value: Value,
    ) -> Result<Value, McpError> {
        if !learning_sidecar_tool(canonical) {
            return Ok(value);
        }
        let paths = collect_learning_candidate_paths(&input, &value);
        if paths.is_empty() {
            return Ok(value);
        }

        let host = Arc::clone(&self.host);
        let caps = self.learning_caps;
        let join =
            tokio::task::spawn_blocking(move || search_learning_reminders(&*host, &paths, caps))
                .await;
        let reminders = match join {
            Ok(Ok(reminders)) => reminders,
            Ok(Err(error)) => {
                tracing::warn!(
                    target: "orbit.mcp.learnings",
                    error = %error,
                    "failed to search learning sidecar",
                );
                Vec::new()
            }
            Err(error) => {
                tracing::warn!(
                    target: "orbit.mcp.learnings",
                    error = %error,
                    "learning sidecar worker failed",
                );
                Vec::new()
            }
        };
        if reminders.is_empty() {
            return Ok(value);
        }

        let admitted = self.admit_learning_reminders(reminders).await?;
        Ok(attach_learning_sidecar(value, admitted))
    }

    async fn admit_learning_reminders(
        &self,
        reminders: Vec<LearningReminder>,
    ) -> Result<Vec<LearningReminder>, McpError> {
        let key = self.learning_session_key();
        let caps = self.learning_caps;
        if let Some(session_id) = self.learning_session_id.clone() {
            let root = std::env::current_dir().map_err(|error| {
                McpError::internal_error(
                    format!("resolve current dir for learning session: {error}"),
                    None,
                )
            })?;
            let path = learning_session_state_path(&root, &session_id);
            let reminders_for_file = reminders.clone();
            let join = tokio::task::spawn_blocking(move || {
                update_learning_session_state(&path, |state| {
                    state.admit_reminders(&reminders_for_file, caps)
                })
            })
            .await
            .map_err(|error| {
                McpError::internal_error(
                    format!("learning session state worker failed: {error}"),
                    None,
                )
            })?;
            let (state, admitted) = join.map_err(|error| {
                McpError::internal_error(format!("update learning session state: {error}"), None)
            })?;
            let mut states = self.learning_states.lock().await;
            states.insert(key, state);
            return Ok(admitted);
        }

        let mut states = self.learning_states.lock().await;
        let state = states.entry(key).or_default();
        Ok(state.admit_reminders(&reminders, caps))
    }

    fn learning_session_key(&self) -> String {
        self.learning_session_id
            .clone()
            .unwrap_or_else(|| PROCESS_LEARNING_SESSION_KEY.to_string())
    }
}

const PROCESS_LEARNING_SESSION_KEY: &str = "__process__";

fn load_learning_state_for_session(session_id: &str) -> Option<LearningInjectionState> {
    let root = std::env::current_dir().ok()?;
    let path = learning_session_state_path(&root, session_id);
    read_learning_session_state(&path).ok().flatten()
}

fn learning_sidecar_tool(canonical: &str) -> bool {
    matches!(
        canonical,
        "orbit.graph.show" | "orbit.graph.refs" | "orbit.task.show"
    )
}

fn collect_learning_candidate_paths(input: &Value, response: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_paths_from_input(input, &mut paths);
    collect_paths_from_response(response, &mut paths);
    paths
}

fn collect_paths_from_input(value: &Value, out: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in ["selector", "selectors", "path", "paths"] {
        if let Some(value) = object.get(key) {
            collect_path_values(value, out);
        }
    }
}

fn collect_paths_from_response(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "file" | "path" | "context_files") {
                    collect_path_values(value, out);
                    continue;
                }
                if key == "code_refs" {
                    collect_code_ref_paths(value, out);
                    continue;
                }
                collect_paths_from_response(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_response(item, out);
            }
        }
        _ => {}
    }
}

fn collect_code_ref_paths(value: &Value, out: &mut Vec<String>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for item in items {
        if let Some(file) = item.get("file") {
            collect_path_values(file, out);
        }
    }
}

fn collect_path_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(raw) => push_candidate_path(raw, out),
        Value::Array(items) => {
            for item in items {
                collect_path_values(item, out);
            }
        }
        _ => {}
    }
}

fn push_candidate_path(raw: &str, out: &mut Vec<String>) {
    let Ok(path) = anchor_path(raw) else {
        return;
    };
    let path = path.to_string_lossy().replace('\\', "/");
    if !path.is_empty() && !out.iter().any(|existing| existing == &path) {
        out.push(path);
    }
}

#[derive(Debug, Clone)]
struct ReminderCandidate {
    reminder: LearningReminder,
    priority: Option<u8>,
    updated_at: String,
}

fn search_learning_reminders(
    host: &dyn McpHost,
    paths: &[String],
    caps: LearningInjectionCaps,
) -> Result<Vec<LearningReminder>, OrbitError> {
    let mut by_id: BTreeMap<String, ReminderCandidate> = BTreeMap::new();
    for path in paths {
        let value = host.call_tool(
            "orbit.learning.search",
            json!({
                "path": path,
                "limit": caps.per_call,
            }),
        )?;
        for candidate in parse_learning_search_candidates(&value) {
            by_id
                .entry(candidate.reminder.id.clone())
                .or_insert(candidate);
        }
    }
    let mut candidates: Vec<_> = by_id.into_values().collect();
    candidates.sort_by(|a, b| {
        priority_rank(b.priority)
            .cmp(&priority_rank(a.priority))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.reminder.id.cmp(&b.reminder.id))
    });
    candidates.truncate(caps.per_call);
    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.reminder)
        .collect())
}

fn parse_learning_search_candidates(value: &Value) -> Vec<ReminderCandidate> {
    let items = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .into_iter()
        .flatten();
    items
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let summary = item.get("summary").and_then(Value::as_str)?.to_string();
            let priority = item
                .get("priority")
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok());
            let updated_at = item
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(ReminderCandidate {
                reminder: LearningReminder {
                    id,
                    summary,
                    comments: Vec::new(),
                },
                priority,
                updated_at,
            })
        })
        .collect()
}

fn priority_rank(priority: Option<u8>) -> i16 {
    priority.map(i16::from).unwrap_or(-1)
}

fn attach_learning_sidecar(mut value: Value, reminders: Vec<LearningReminder>) -> Value {
    if reminders.is_empty() {
        return value;
    }
    let sidecar = Value::Array(
        reminders
            .into_iter()
            .map(|reminder| {
                json!({
                    "id": reminder.id,
                    "summary": reminder.summary,
                })
            })
            .collect(),
    );
    match &mut value {
        Value::Object(object) => {
            object.insert("learnings".to_string(), sidecar);
            value
        }
        _ => json!({
            "result": value,
            "learnings": sidecar,
        }),
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

const TASK_TYPE_ENUM: &[&str] = &["feature", "bug", "refactor", "chore"];

const TASK_ADD_STATUS_ENUM: &[&str] = &[
    "proposed",
    "backlog",
    "someday",
    "in-progress",
    "review",
    "done",
    "blocked",
    "rejected",
];

const TASK_UPDATE_STATUS_ENUM: &[&str] = &[
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
        ("orbit.task.update", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.add", "status") => Some(TASK_ADD_STATUS_ENUM),
        ("orbit.task.update", "status") => Some(TASK_UPDATE_STATUS_ENUM),
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
    let key = param_type.trim().to_ascii_lowercase();
    match key.as_str() {
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
        "object_list" | "object[]" | "objects" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "array", "items": { "type": "object" } },
                    { "type": "string" },
                ]),
            );
        }
        _ => {
            tracing::warn!(
                target: "orbit.mcp.adapter",
                param_type = %param_type,
                "unknown ToolParam type degrading to string"
            );
            m.insert("type".to_string(), Value::String("string".to_string()));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

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

    fn request_with_args(name: &str, args: Value) -> CallToolRequestParams {
        CallToolRequestParams::new(sanitize_tool_name(name)).with_arguments(
            args.as_object()
                .expect("object args")
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }

    #[test]
    fn task_add_schema_excludes_legacy_friction_enums() {
        let schema = build_input_schema("orbit.task.add", &[param("type"), param("status")]);
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties");

        let type_enum = properties["type"]["enum"].as_array().expect("type enum");
        assert!(!type_enum.iter().any(|value| value == "friction"));

        let status_enum = properties["status"]["enum"]
            .as_array()
            .expect("status enum");
        assert!(!status_enum.iter().any(|value| value == "friction"));
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

    #[tokio::test]
    async fn learning_sidecar_present_with_summary_only_on_path_match() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert(
            "crates/orbit-engine/src/lib.rs".to_string(),
            vec![json!({
                "id": "L20260515-0001",
                "summary": "Remember the engine rule.",
                "body": "full body must stay out",
                "updated_at": "2026-05-15T00:00:00Z",
                "priority": 7
            })],
        );
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
            }),
            search_by_path,
        ));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            LearningInjectionState::default(),
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.graph.show",
                json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert_eq!(
            structured.get("learnings"),
            Some(&json!([{
                "id": "L20260515-0001",
                "summary": "Remember the engine rule."
            }]))
        );
        assert!(
            !serde_json::to_string(structured)
                .expect("json")
                .contains("full body")
        );
    }

    #[tokio::test]
    async fn learning_sidecar_absent_when_no_learning_matches() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert("crates/orbit-engine/src/lib.rs".to_string(), Vec::new());
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "code_refs": [{"file": "crates/orbit-engine/src/lib.rs"}]
            }),
            search_by_path,
        ));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            LearningInjectionState::default(),
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.graph.refs",
                json!({"selector": "file:crates/orbit-engine/src/lib.rs"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(structured.get("learnings").is_none());
    }

    #[tokio::test]
    async fn l1_seeded_learning_is_suppressed_by_l2_dedup_state() {
        let mut search_by_path = HashMap::new();
        search_by_path.insert(
            "crates/orbit-engine/src/lib.rs".to_string(),
            vec![json!({
                "id": "L20260515-0001",
                "summary": "Already injected at L1.",
                "updated_at": "2026-05-15T00:00:00Z",
                "priority": null
            })],
        );
        let host = Arc::new(LearningSidecarHost::new(
            json!({
                "context_files": ["file:crates/orbit-engine/src/lib.rs"]
            }),
            search_by_path,
        ));
        let initial_state = LearningInjectionState::seeded(["L20260515-0001".to_string()]);
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps::default(),
            initial_state,
        );

        let result = server
            .call_tool_request(request_with_args(
                "orbit.task.show",
                json!({"id": "ORB-00009"}),
            ))
            .await
            .expect("call succeeds");
        let structured = result
            .structured_content
            .as_ref()
            .expect("structured content");

        assert!(structured.get("learnings").is_none());
        let states = server.learning_states.lock().await;
        let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
        assert_eq!(state.count, 1);
        assert!(state.emitted_ids.contains("L20260515-0001"));
    }

    #[tokio::test]
    async fn learning_sidecar_enforces_per_session_hard_cap() {
        let mut search_by_path = HashMap::new();
        for call_idx in 0..5 {
            let path = format!("p{call_idx}.rs");
            let rows: Vec<_> = (0..5)
                .map(|row_idx| {
                    let id_idx = call_idx * 5 + row_idx;
                    json!({
                        "id": format!("L20260515-{id_idx:04}"),
                        "summary": format!("Learning {id_idx}"),
                        "updated_at": "2026-05-15T00:00:00Z",
                        "priority": null
                    })
                })
                .collect();
            search_by_path.insert(path, rows);
        }
        let host = Arc::new(LearningSidecarHost::new(json!({}), search_by_path));
        let server = OrbitToolServer::new_for_test(
            host,
            None,
            LearningInjectionCaps {
                per_call: 5,
                per_session_hard: 20,
            },
            LearningInjectionState::default(),
        );
        let mut emitted = 0usize;

        for call_idx in 0..5 {
            let result = server
                .call_tool_request(request_with_args(
                    "orbit.graph.show",
                    json!({"selector": format!("file:p{call_idx}.rs")}),
                ))
                .await
                .expect("call succeeds");
            let structured = result
                .structured_content
                .as_ref()
                .expect("structured content");
            emitted += structured
                .get("learnings")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
        }

        assert_eq!(emitted, 20);
        let states = server.learning_states.lock().await;
        let state = states.get(PROCESS_LEARNING_SESSION_KEY).expect("state");
        assert_eq!(state.count, 20);
        assert_eq!(state.emitted_ids.len(), 20);
    }

    struct LearningSidecarHost {
        response: Value,
        search_by_path: HashMap<String, Vec<Value>>,
        calls: StdMutex<Vec<String>>,
    }

    impl LearningSidecarHost {
        fn new(response: Value, search_by_path: HashMap<String, Vec<Value>>) -> Self {
            Self {
                response,
                search_by_path,
                calls: StdMutex::new(Vec::new()),
            }
        }
    }

    impl crate::McpHost for LearningSidecarHost {
        fn list_tool_schemas(&self) -> Vec<ToolSchema> {
            vec![
                tool_schema("orbit.graph.show"),
                tool_schema("orbit.graph.refs"),
                tool_schema("orbit.task.show"),
                tool_schema("orbit.learning.search"),
            ]
        }

        fn call_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(name.to_string());
            if name == "orbit.learning.search" {
                let path = input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return Ok(Value::Array(
                    self.search_by_path.get(path).cloned().unwrap_or_default(),
                ));
            }
            Ok(self.response.clone())
        }
    }

    // --- ORB-00102 tests: object_list schema + loud fallback + e2e via MCP adapter ---

    fn capture_warnings<F, T>(f: F) -> (T, String)
    where
        F: FnOnce() -> T,
    {
        use std::io::{self, Write};
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::filter::LevelFilter;
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone)]
        struct CaptureMakeWriter(Arc<Mutex<Vec<u8>>>);
        struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

        impl<'a> MakeWriter<'a> for CaptureMakeWriter {
            type Writer = CaptureWriter;
            fn make_writer(&'a self) -> Self::Writer {
                CaptureWriter(Arc::clone(&self.0))
            }
        }
        impl Write for CaptureWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().expect("capture lock").extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureMakeWriter(Arc::clone(&buffer)))
            .with_max_level(LevelFilter::WARN)
            .with_target(true)
            .with_ansi(false)
            .without_time()
            .finish();
        let result = tracing::subscriber::with_default(subscriber, f);
        let logs = String::from_utf8(buffer.lock().expect("capture buffer lock").clone())
            .expect("utf8 logs");
        (result, logs)
    }

    #[test]
    fn property_for_object_list_emits_anyof_array_of_objects_or_string() {
        for token in [
            "object_list",
            "object[]",
            "objects",
            "OBJECT_LIST",
            "object[] ",
        ] {
            let prop = property_for(token);
            let any_of = match prop.get("anyOf").and_then(Value::as_array) {
                Some(any_of) => any_of,
                None => panic!("anyOf present for {token}"),
            };
            let has_array_objects = any_of.iter().any(|s| {
                s.get("type").and_then(Value::as_str) == Some("array")
                    && s.get("items")
                        .and_then(|i| i.get("type"))
                        .and_then(Value::as_str)
                        == Some("object")
            });
            let has_string = any_of
                .iter()
                .any(|s| s.get("type").and_then(Value::as_str) == Some("string"));
            assert!(has_array_objects, "{token} must accept array-of-objects");
            assert!(has_string, "{token} must accept string fallback");
        }
    }

    #[test]
    fn property_for_unknown_emits_tracing_warn_at_target() {
        let token = "<unknown-token-not-in-match-arms>";
        let (prop, logs) = capture_warnings(|| property_for(token));
        assert_eq!(
            prop.get("type").and_then(Value::as_str),
            Some("string"),
            "fallback still produces string"
        );
        assert!(
            logs.contains("unknown ToolParam type degrading to string"),
            "warning message present: {logs}"
        );
        assert!(logs.contains("orbit.mcp.adapter"), "target present: {logs}");
        assert!(
            logs.contains(token),
            "offending token named in event: {logs}"
        );
    }

    #[test]
    fn learning_add_schema_advertises_object_list_shape_for_evidence() {
        let params = vec![
            param_with_type("summary", "string"),
            param_with_type("scope", "object"),
            param_with_type("evidence", "object_list"),
            param_with_type("model", "string"),
        ];
        let schema = build_input_schema("orbit.learning.add", &params);
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties");
        let ev = properties
            .get("evidence")
            .and_then(Value::as_object)
            .expect("evidence property");
        assert!(
            ev.get("anyOf").is_some(),
            "evidence must use anyOf (array-of-object | string), got: {ev:?}"
        );
        // must not be the old silent string
        assert_ne!(
            ev.get("type").and_then(Value::as_str),
            Some("string"),
            "evidence must not degrade to plain string"
        );
    }

    /// Simple in-memory persistence host for e2e MCP learning add/update/show tests.
    /// Verifies that array-shaped evidence reaches the handler (proving schema allows it).
    struct LearningPersistenceHost {
        store: StdMutex<HashMap<String, Value>>,
        next: StdMutex<u32>,
    }

    impl LearningPersistenceHost {
        fn new() -> Self {
            Self {
                store: StdMutex::new(HashMap::new()),
                next: StdMutex::new(0),
            }
        }
        fn next_id(&self) -> String {
            let mut n = self.next.lock().expect("next lock");
            *n += 1;
            format!("L-test-{:04}", *n)
        }
    }

    impl crate::McpHost for LearningPersistenceHost {
        fn list_tool_schemas(&self) -> Vec<ToolSchema> {
            vec![
                tool_schema("orbit.learning.add"),
                tool_schema("orbit.learning.update"),
                tool_schema("orbit.learning.show"),
            ]
        }

        fn call_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
            let canonical = if name.contains("learning.add") {
                "orbit.learning.add"
            } else if name.contains("learning.update") {
                "orbit.learning.update"
            } else if name.contains("learning.show") {
                "orbit.learning.show"
            } else {
                name
            };
            match canonical {
                "orbit.learning.add" => {
                    let id = self.next_id();
                    let mut rec = input.clone();
                    if let Some(obj) = rec.as_object_mut() {
                        obj.insert("id".to_string(), Value::String(id.clone()));
                        obj.insert(
                            "created_at".to_string(),
                            Value::String("2026-05-17T12:00:00Z".to_string()),
                        );
                        if !obj.contains_key("evidence") {
                            obj.insert("evidence".to_string(), Value::Array(vec![]));
                        }
                    }
                    self.store
                        .lock()
                        .expect("store lock")
                        .insert(id.clone(), rec.clone());
                    Ok(rec)
                }
                "orbit.learning.update" => {
                    let id = input
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let mut guard = self.store.lock().expect("store lock");
                    if let Some(existing) = guard.get_mut(&id) {
                        if let (Some(obj), Some(up)) = (existing.as_object_mut(), input.as_object())
                        {
                            for (k, v) in up.iter() {
                                if k != "id" {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        Ok(existing.clone())
                    } else {
                        Ok(json!({ "id": id, "updated": false }))
                    }
                }
                "orbit.learning.show" => {
                    let id = input
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let guard = self.store.lock().expect("store lock");
                    if let Some(rec) = guard.get(&id) {
                        Ok(rec.clone())
                    } else {
                        Ok(json!({ "id": id, "found": false }))
                    }
                }
                _ => Ok(json!({ "ok": true, "echo": name })),
            }
        }
    }

    #[tokio::test]
    async fn orbit_learning_add_via_mcp_adapter_accepts_evidence_array() {
        let host = Arc::new(LearningPersistenceHost::new());
        let server = OrbitToolServer::new(host);

        let evidence = json!([{ "kind": "task", "ref": "T-test" }]);
        let req = request_with_args(
            "orbit.learning.add",
            json!({
                "summary": "MCP evidence array test",
                "scope": { "tags": ["mcp-test"] },
                "evidence": evidence,
                "model": "grok"
            }),
        );
        let res = server
            .call_tool_request(req)
            .await
            .expect("MCP call to learning.add succeeds");
        let body = res.structured_content.expect("structured response");
        let id = body.get("id").and_then(Value::as_str).expect("created id");

        // re-fetch via show (exercises round-trip)
        let show_req = request_with_args("orbit.learning.show", json!({ "id": id }));
        let show_res = server
            .call_tool_request(show_req)
            .await
            .expect("show after add");
        let shown = show_res.structured_content.expect("shown record");
        let got_ev = shown
            .get("evidence")
            .and_then(Value::as_array)
            .expect("evidence persisted as array");
        assert_eq!(got_ev.len(), 1, "one evidence entry");
        assert_eq!(got_ev[0]["kind"], "task");
        assert_eq!(got_ev[0]["ref"], "T-test");
        // response shape has the fields show would return
        assert!(shown.get("id").is_some());
        assert!(shown.get("created_at").is_some() || shown.get("updated_at").is_some());
    }

    #[tokio::test]
    async fn orbit_learning_update_via_mcp_adapter_accepts_evidence_array_live_repro() {
        let host = Arc::new(LearningPersistenceHost::new());
        let server = OrbitToolServer::new(host);

        // seed via add
        let seed = request_with_args(
            "orbit.learning.add",
            json!({
                "summary": "for update repro",
                "scope": { "tags": ["repro"] },
                "model": "claude"
            }),
        );
        let seed_res = server.call_tool_request(seed).await.expect("seed add");
        let seed_id = seed_res
            .structured_content
            .expect("seed body")
            .get("id")
            .and_then(Value::as_str)
            .expect("seed id")
            .to_string();

        // now the live repro: update evidence via MCP (the F2026-05-025 case)
        let new_evidence = json!([{ "kind": "task", "ref": "ORB-00022" }]);
        let upd_req = request_with_args(
            "orbit.learning.update",
            json!({
                "id": seed_id,
                "model": "claude",
                "evidence": new_evidence
            }),
        );
        let upd_res = server
            .call_tool_request(upd_req)
            .await
            .expect("update via MCP must succeed (was failing before fix)");
        let _updated = upd_res.structured_content.expect("update response");

        // verify by show
        let show_req = request_with_args("orbit.learning.show", json!({ "id": seed_id }));
        let shown = server
            .call_tool_request(show_req)
            .await
            .expect("show after update")
            .structured_content
            .expect("shown");
        let ev = shown
            .get("evidence")
            .and_then(Value::as_array)
            .expect("evidence after update is array");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["ref"], "ORB-00022");
        assert_eq!(ev[0]["kind"], "task");
    }
}
