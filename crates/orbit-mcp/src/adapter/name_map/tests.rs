use super::*;
use serde_json::Value;

use super::super::test_support::tool_schema;

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
