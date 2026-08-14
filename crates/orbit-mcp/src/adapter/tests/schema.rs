use std::sync::Arc;

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, McpToolScope, McpTransport,
    ToolParam, ToolSchema, ToolSessionContext,
};
use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams, Meta};

use super::super::dispatch::session_context_from_initialize;
use super::super::schema::{build_input_schema, property_for, schema_to_tool};
use serde_json::{Value, json};

use super::super::OrbitToolServer;
use super::super::test_support::{
    SessionContextHost, param, param_with_type, request_with_args, tool_schema,
};

#[test]
fn generic_task_schema_is_structural_and_owns_no_domain_enums() {
    let schema = build_input_schema("orbit.task.add", &[param("type"), param("status")]);
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties");

    assert!(properties["type"].get("enum").is_none());
    assert!(properties["status"].get("enum").is_none());
}

#[test]
fn generic_task_update_schema_is_structural() {
    let schema = build_input_schema("orbit.task.update", &[param("status")]);
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties");
    assert!(properties["status"].get("enum").is_none());
}

#[test]
fn schema_to_tool_keeps_dotted_orbit_tools_advertised_with_underscores() {
    let schema = tool_schema("orbit.task.add");
    let input_schema = build_input_schema(&schema.name, &schema.parameters);
    let tool = schema_to_tool(schema, input_schema);
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

/// A handler-required list parameter must be advertised in `required` as a
/// string|array union. Otherwise a schema-following caller can omit it and the
/// backend rejects the call only after dispatch.
#[test]
fn required_string_list_param_is_advertised_as_string_or_array() {
    let selectors = ToolParam {
        name: "selectors".to_string(),
        description: "Selector string or array.".to_string(),
        param_type: "string_list".to_string(),
        required: true,
    };
    let summary = param_with_type("summary", "boolean");
    let schema = build_input_schema("fixture.batch", &[selectors, summary]);

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

// --- ORB-10448 / F2026-07-099: the broker's workspace selector must be
// advertised on every workspace-scoped tool. A managed executor speaks through
// a general-purpose MCP client that cannot inject initialize `_meta`, so the
// call argument is its only routing surface.

fn definition_with_scope(
    name: &str,
    parameters: Vec<ToolParam>,
    scope: McpToolScope,
) -> McpToolDefinition {
    let schema = ToolSchema {
        name: name.to_string(),
        description: String::new(),
        parameters,
        builtin: true,
    };
    let policy = McpToolPolicy::agent_and_operator(McpToolPlacement::Owner).with_scope(scope);
    McpToolDefinition::new(schema, policy).expect("test definition policy is valid")
}

fn advertised_properties(definition: &McpToolDefinition) -> serde_json::Map<String, Value> {
    let server = OrbitToolServer::new(Arc::new(SessionContextHost::default()));
    let schema = server
        .input_schema_for(definition)
        .expect("input schema resolves");
    schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .expect("properties object")
}

#[test]
fn workspace_scoped_tool_advertises_the_broker_workspace_selector() {
    let definition = definition_with_scope(
        "orbit.task.show",
        vec![param("id")],
        McpToolScope::WorkspaceRequired,
    );

    let properties = advertised_properties(&definition);
    let workspace = properties
        .get("workspace")
        .and_then(Value::as_object)
        .expect("workspace selector advertised on a workspace-scoped tool");

    assert_eq!(
        workspace.get("type").and_then(Value::as_str),
        Some("string")
    );
    let description = workspace
        .get("description")
        .and_then(Value::as_str)
        .expect("selector carries routing guidance");
    assert!(
        description.contains("registered workspace name"),
        "selector must document the registered-name form: {description}"
    );
    assert!(
        description.contains("ws_*"),
        "selector must document logical ws_* ids: {description}"
    );
    assert!(
        description.contains("absolute path to a local checkout"),
        "selector must document the checkout-path form: {description}"
    );
    assert!(
        description.contains("worktree"),
        "selector must state that a linked worktree resolves: {description}"
    );
}

#[test]
fn global_scoped_tool_does_not_advertise_a_workspace_selector() {
    let definition = definition_with_scope(
        "orbit.workspace.list",
        vec![param("limit")],
        McpToolScope::Global,
    );

    let properties = advertised_properties(&definition);

    assert!(
        !properties.contains_key("workspace"),
        "a global tool routes without a workspace: {properties:?}"
    );
}

#[test]
fn tool_declaring_its_own_workspace_param_keeps_that_description() {
    let declared = ToolParam {
        name: "workspace".to_string(),
        description: "Workspace path for the task".to_string(),
        param_type: "string".to_string(),
        required: false,
    };
    let definition = definition_with_scope(
        "orbit.task.add",
        vec![param("title"), declared],
        McpToolScope::WorkspaceRequired,
    );

    let properties = advertised_properties(&definition);

    assert_eq!(
        properties["workspace"]
            .get("description")
            .and_then(Value::as_str),
        Some("Workspace path for the task"),
        "injection must not overwrite a tool's own selector documentation"
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
            "workspace": " /repo/main ",
            "workspace_id": "spoofed-workspace",
            "caller_machine_id": "spoofed-caller",
            "process_machine_id": "spoofed-process",
            "transport": "ssh-mcp",
            "effective_capabilities": ["operator", "runner"],
            "origin_session_id": "spoofed-session",
            "mcp_call_id": "spoofed-call",
            "leased_run": {"run_id": "spoofed-run", "lease_id": "spoofed-lease"},
            "role": "admin",
            "model": "spoofed-model",
            "task_id": "spoofed-task"
        },
        "orbit.workspace_id": "also-spoofed"
    }));

    let session_context = session_context_from_initialize(&params, &Meta::new());

    assert_eq!(session_context.workspace.as_deref(), Some("/repo/main"));
    assert_eq!(session_context.workspace_id, None);
    assert_eq!(session_context.caller_machine_id, None);
    assert_eq!(session_context.process_machine_id, None);
    assert_eq!(session_context.transport, None);
    assert!(session_context.effective_capabilities.is_empty());
    assert_eq!(session_context.origin_session_id, None);
    assert_eq!(session_context.mcp_call_id, None);
}

#[test]
fn initialize_transport_meta_extracts_orbit_workspace_session_context() {
    // Over a real transport rmcp strips `_meta` from the params and delivers
    // it through the request context instead; the params-level field stays
    // `None`. The announced session workspace must still be honored.
    let params = InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("orbit-test-client", "0"),
    );
    let Value::Object(meta_object) = json!({
        "orbit": {
            "workspace": " /repo/main "
        }
    }) else {
        panic!("test meta must be an object");
    };

    let session_context = session_context_from_initialize(&params, &Meta(meta_object));

    assert_eq!(session_context.workspace.as_deref(), Some("/repo/main"));
}

#[test]
fn initialize_params_meta_wins_over_transport_meta() {
    let params = initialize_params_with_meta(json!({
        "orbit": { "workspace": "/repo/params" }
    }));
    let Value::Object(meta_object) = json!({
        "orbit": { "workspace": "/repo/transport" }
    }) else {
        panic!("test meta must be an object");
    };

    let session_context = session_context_from_initialize(&params, &Meta(meta_object));

    assert_eq!(session_context.workspace.as_deref(), Some("/repo/params"));
}

#[tokio::test]
async fn mcp_session_context_reaches_tool_calls_without_workspace_input() {
    let host = Arc::new(SessionContextHost::default());
    let server = OrbitToolServer::new(host.clone());
    let mut trusted_context = ToolSessionContext::trusted_local(
        Some("ws_orbit".to_string()),
        Some("hm_local".to_string()),
        Some("local-host".to_string()),
    );
    trusted_context.workspace = Some("/repo/main".to_string());
    trusted_context.origin_session_id = Some("mcp-session-shared".to_string());
    server.replace_session_context(trusted_context);

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
    assert_eq!(calls[0].2.workspace_id.as_deref(), Some("ws_orbit"));
    assert_eq!(calls[0].2.transport, Some(McpTransport::Local));
    assert_eq!(
        calls[0].2.effective_capabilities,
        [McpCapability::Agent].into_iter().collect()
    );
    assert_eq!(
        calls[0].2.origin_session_id.as_deref(),
        Some("mcp-session-shared")
    );
    assert_eq!(calls[1].2.origin_session_id, calls[0].2.origin_session_id);
    assert!(calls[0].2.mcp_call_id.is_some());
    assert!(calls[1].2.mcp_call_id.is_some());
    assert_ne!(calls[0].2.mcp_call_id, calls[1].2.mcp_call_id);
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

/// ORB-00234/ORB-00255: MCP schema for orbit_task_add advertises the trimmed
/// create-task fields with correct enums (verifiable via debug surfaces or this
/// direct build).
#[test]
fn task_add_structural_schema_exposes_trimmed_fields_without_domain_enums() {
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
        param_with_type("crew", "string"),
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
            "crew",
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

    assert!(properties["complexity"].get("enum").is_none());
    assert!(properties["model"].get("enum").is_none());

    for removed in [
        "plan",
        "status",
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

    // crew was un-retired (ORB-10123): it is now an authoring param again.
    assert!(
        properties.contains_key("crew"),
        "crew must appear in MCP schema properties for orbit.task.add"
    );
}
