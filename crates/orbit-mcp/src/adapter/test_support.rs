use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn param_with_type(name: &str, param_type: &str) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: String::new(),
        param_type: param_type.to_string(),
        required: false,
    }
}

pub(super) fn param(name: &str) -> ToolParam {
    param_with_type(name, "string")
}

pub(super) fn tool_schema(name: &str) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: String::new(),
        parameters: Vec::new(),
        builtin: true,
    }
}

pub(super) fn request_with_args(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(sanitize_tool_name(name)).with_arguments(
        args.as_object()
            .expect("object args")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

pub(super) struct StubHost {
    pub(super) schemas: Vec<ToolSchema>,
}

impl crate::McpHost for StubHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        self.schemas.clone()
    }

    fn call_tool(&self, _name: &str, _input: Value) -> Result<Value, OrbitError> {
        Ok(Value::Null)
    }
}

pub(super) struct EchoArrayHost {
    pub(super) schemas: Vec<ToolSchema>,
}

impl crate::McpHost for EchoArrayHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        self.schemas.clone()
    }

    fn call_tool(&self, name: &str, _input: Value) -> Result<Value, OrbitError> {
        Ok(json!([{ "tool": name }]))
    }
}

pub(super) struct LearningSidecarHost {
    response: Value,
    search_by_path: HashMap<String, Vec<Value>>,
    calls: StdMutex<Vec<String>>,
}

impl LearningSidecarHost {
    pub(super) fn new(response: Value, search_by_path: HashMap<String, Vec<Value>>) -> Self {
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

/// Simple in-memory persistence host for e2e MCP learning add/update/show tests.
/// Verifies that array-shaped evidence reaches the handler (proving schema allows it).
pub(super) struct LearningPersistenceHost {
    store: StdMutex<HashMap<String, Value>>,
    next: StdMutex<u32>,
}

impl LearningPersistenceHost {
    pub(super) fn new() -> Self {
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
                    if let (Some(obj), Some(up)) = (existing.as_object_mut(), input.as_object()) {
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
