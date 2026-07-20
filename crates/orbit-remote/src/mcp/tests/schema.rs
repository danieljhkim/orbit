use orbit_common::types::McpToolDefinition;
use serde_json::Value;

use super::super::host::canonical_mcp_tool_definitions;
use super::super::schema::remote_input_schema;

#[test]
fn task_enum_metadata_is_owned_by_remote_schema_composition() {
    let definitions = canonical_mcp_tool_definitions().expect("canonical definitions");
    let add = definition(&definitions, "orbit.task.add");
    let update = definition(&definitions, "orbit.task.update");

    let add_schema = remote_input_schema(add).expect("task add schema");
    assert_enum(
        &add_schema,
        "type",
        &["feature", "bug", "refactor", "chore"],
    );
    assert_enum(&add_schema, "complexity", &["low", "medium", "hard"]);
    assert_enum(&add_schema, "model", &["codex", "claude", "gemini", "grok"]);

    let update_schema = remote_input_schema(update).expect("task update schema");
    assert_enum(
        &update_schema,
        "status",
        &[
            "proposed",
            "backlog",
            "someday",
            "in-progress",
            "review",
            "done",
            "blocked",
            "rejected",
        ],
    );
    assert!(
        update_schema["properties"]["status"]["enum"]
            .as_array()
            .expect("status enum")
            .iter()
            .all(|value| value != "friction")
    );
}

#[test]
fn human_knowledge_maintenance_is_never_advertised_as_an_mcp_tool() {
    let definitions = canonical_mcp_tool_definitions().expect("canonical definitions");
    let names = definitions
        .iter()
        .map(|definition| definition.schema.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for forbidden in [
        "orbit.knowledge.allocate",
        "orbit.knowledge.sync",
        "orbit.learning.allocate",
        "orbit.adr.allocate",
    ] {
        assert!(!names.contains(forbidden), "advertised {forbidden}");
    }
}

fn definition<'a>(definitions: &'a [McpToolDefinition], name: &str) -> &'a McpToolDefinition {
    definitions
        .iter()
        .find(|definition| definition.schema.name == name)
        .unwrap_or_else(|| panic!("missing definition {name}"))
}

fn assert_enum(schema: &serde_json::Map<String, Value>, property: &str, expected: &[&str]) {
    let actual = schema["properties"][property]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("missing enum for {property}"));
    assert_eq!(
        actual,
        &expected
            .iter()
            .map(|value| Value::String((*value).to_string()))
            .collect::<Vec<_>>()
    );
}
