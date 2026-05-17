use std::sync::Arc;

use orbit_common::types::{ToolParam, ToolSchema};
use rmcp::model::{JsonObject, Tool};
use serde_json::{Map, Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn schema_to_tool(schema: ToolSchema) -> Tool {
    let description = schema.description.clone();
    let input_schema = build_input_schema(&schema.name, &schema.parameters);
    let advertised_name = sanitize_tool_name(&schema.name);
    Tool::new(advertised_name, description, Arc::new(input_schema))
}

pub(super) fn build_input_schema(tool_name: &str, params: &[ToolParam]) -> JsonObject {
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

pub(super) fn enum_values_for(
    tool_name: &str,
    param_name: &str,
) -> Option<&'static [&'static str]> {
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
pub(super) fn property_for(param_type: &str) -> Map<String, Value> {
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
    use serde_json::{Value, json};

    use super::super::OrbitToolServer;
    use super::super::test_support::{
        LearningPersistenceHost, param, param_with_type, request_with_args, tool_schema,
    };

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
    fn schema_to_tool_keeps_dotted_orbit_tools_advertised_with_underscores() {
        let tool = schema_to_tool(tool_schema("orbit.task.add"));
        assert_eq!(tool.name.as_ref(), "orbit_task_add");
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
