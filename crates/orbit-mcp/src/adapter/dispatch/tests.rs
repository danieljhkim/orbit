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
        "orbit.task.review_thread.list",
        "orbit.learning.list",
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
