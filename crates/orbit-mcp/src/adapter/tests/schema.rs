use std::sync::Arc;

use orbit_common::types::{ToolParam, ToolSessionContext};
use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams, Meta};

use super::super::dispatch::session_context_from_initialize;
use super::super::schema::{build_input_schema, property_for, schema_to_tool};
use serde_json::{Value, json};

use super::super::OrbitToolServer;
use super::super::test_support::{
    LearningPersistenceHost, SessionContextHost, param, param_with_type, request_with_args,
    tool_schema,
};

#[test]
fn task_add_schema_excludes_legacy_friction_and_status_enums() {
    let schema = build_input_schema("orbit.task.add", &[param("type"), param("status")]);
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties");

    let type_enum = properties["type"]["enum"].as_array().expect("type enum");
    assert!(!type_enum.iter().any(|value| value == "friction"));

    assert!(
        properties["status"].get("enum").is_none(),
        "orbit.task.add no longer advertises status at all"
    );
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
    let schema = build_input_schema(
        "orbit.task.update",
        &[param_with_type("dependencies", "string_list")],
    );
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
        "orbit.task.update dependencies must accept an array of strings"
    );
    assert!(
        any_of
            .iter()
            .any(|schema| schema.get("type").and_then(Value::as_str) == Some("string")),
        "orbit.task.update dependencies must accept a string"
    );
}

/// ORB-00382: `orbit.graph.pack`'s handler requires `selectors`, so the
/// published MCP schema must advertise it in `required` (as a string|array
/// union) — otherwise a caller following the schema omits it and the backend
/// rejects the call with `missing selectors`. This pins the adapter contract
/// that a handler-required param surfaces in the published `required` set.
#[test]
fn graph_pack_mcp_schema_marks_required_selectors_as_string_or_array() {
    let selectors = ToolParam {
        name: "selectors".to_string(),
        description: "Graph selector string or array.".to_string(),
        param_type: "string_list".to_string(),
        required: true,
    };
    let summary = param_with_type("summary", "boolean");
    let schema = build_input_schema("orbit.graph.pack", &[selectors, summary]);

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("required array present");
    assert!(
        required.iter().any(|value| value == "selectors"),
        "selectors must be advertised as required: {required:?}"
    );
    assert!(
        !required.iter().any(|value| value == "summary"),
        "optional params must not appear in required: {required:?}"
    );

    // Advertised as a string|array union so a bare string or an array validate.
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties");
    let any_of = properties["selectors"]["anyOf"]
        .as_array()
        .expect("selectors string-list union");
    assert!(
        any_of
            .iter()
            .any(|shape| shape.get("type").and_then(Value::as_str) == Some("array")),
        "selectors must accept an array: {any_of:?}"
    );
    assert!(
        any_of
            .iter()
            .any(|shape| shape.get("type").and_then(Value::as_str) == Some("string")),
        "selectors must accept a bare string: {any_of:?}"
    );
}

fn initialize_params_with_meta(meta: Value) -> InitializeRequestParams {
    let mut params = InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("orbit-test-client", "0"),
    );
    let Value::Object(object) = meta else {
        panic!("test meta must be an object");
    };
    params.meta = Some(Meta(object));
    params
}

#[test]
fn initialize_meta_extracts_orbit_workspace_session_context() {
    let params = initialize_params_with_meta(json!({
        "orbit": {
            "workspace": " /repo/main "
        }
    }));

    let session_context = session_context_from_initialize(&params);

    assert_eq!(session_context.workspace.as_deref(), Some("/repo/main"));
}

#[tokio::test]
async fn mcp_session_context_reaches_tool_calls_without_workspace_input() {
    let host = Arc::new(SessionContextHost::default());
    let server = OrbitToolServer::new(host.clone());
    server.replace_session_context(ToolSessionContext::with_workspace("/repo/main"));

    let explicit = server
        .call_tool_request(request_with_args(
            "orbit.task.list",
            json!({ "workspace": "/repo/main" }),
        ))
        .await
        .expect("explicit workspace call succeeds")
        .structured_content
        .expect("explicit structured content");
    let ambient = server
        .call_tool_request(request_with_args("orbit.task.list", json!({})))
        .await
        .expect("ambient workspace call succeeds")
        .structured_content
        .expect("ambient structured content");

    assert_eq!(ambient, explicit);
    let calls = host.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].2.workspace.as_deref(), Some("/repo/main"));
    assert!(calls[1].1.get("workspace").is_none());
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
    let logs =
        String::from_utf8(buffer.lock().expect("capture buffer lock").clone()).expect("utf8 logs");
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

/// ORB-00234/ORB-00255: MCP schema for orbit_task_add advertises the trimmed
/// create-task fields with correct enums (verifiable via debug surfaces or this
/// direct build).
#[test]
fn task_add_mcp_schema_exposes_trimmed_fields_with_complexity_and_model_enums() {
    // Use representative params that the real add schema includes (the
    // build_input_schema only cares about the ones passed for enum injection).
    let params = vec![
        param_with_type("title", "string"),
        param_with_type("description", "string"),
        param_with_type("workspace", "string"),
        param_with_type("acceptance_criteria", "string_list"),
        param_with_type("tags", "string_list"),
        param_with_type("context_files", "string_list"),
        param_with_type("priority", "string"),
        param_with_type("complexity", "string"),
        param_with_type("type", "string"),
        param_with_type("relations", "array"),
        param_with_type("model", "string"),
    ];
    let schema = build_input_schema("orbit.task.add", &params);
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties object");

    let property_names = properties.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(
        property_names,
        vec![
            "acceptance_criteria",
            "complexity",
            "context_files",
            "description",
            "model",
            "priority",
            "relations",
            "tags",
            "title",
            "type",
            "workspace",
        ]
    );

    // complexity must have the low/medium/hard enum
    let comp = properties.get("complexity").expect("complexity in schema");
    let comp_enum = comp
        .get("enum")
        .and_then(Value::as_array)
        .expect("complexity enum array");
    assert_eq!(
        comp_enum,
        &vec![
            Value::String("low".into()),
            Value::String("medium".into()),
            Value::String("hard".into())
        ]
    );

    // model must have the four families (injected for any tool's model)
    let model = properties.get("model").expect("model in schema");
    let model_enum = model
        .get("enum")
        .and_then(Value::as_array)
        .expect("model enum array");
    assert!(model_enum.iter().any(|v| v == "codex"));
    assert!(model_enum.iter().any(|v| v == "grok"));

    for removed in [
        "plan",
        "status",
        "crew",
        "parent_id",
        "source_task_id",
        "external_refs",
        "context",
        "comment",
        "dependencies",
    ] {
        assert!(
            !properties.contains_key(removed),
            "{removed} must not appear in MCP schema properties for orbit.task.add"
        );
    }
}
