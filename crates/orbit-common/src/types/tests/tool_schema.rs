use serde_json::Value;

use crate::types::{ToolParam, tool_input_schema_for, tool_parameter_schema};

fn param(name: &str, param_type: &str) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: String::new(),
        param_type: param_type.to_string(),
        required: false,
    }
}

#[test]
fn task_metadata_is_shared_by_every_tool_transport() {
    let schema = tool_input_schema_for(
        "orbit.task.add",
        &[
            param("type", "string"),
            param("status", "string"),
            param("complexity", "string"),
            param("model", "string"),
        ],
    );
    let properties = schema["properties"].as_object().expect("properties");

    assert_eq!(
        properties["type"]["enum"],
        json_values(&["feature", "bug", "refactor", "chore"])
    );
    assert_eq!(
        properties["complexity"]["enum"],
        json_values(&["low", "medium", "hard"])
    );
    assert_eq!(
        properties["model"]["enum"],
        json_values(&["codex", "claude", "gemini", "grok"])
    );
    assert!(
        properties["status"]["enum"]
            .as_array()
            .expect("status enum")
            .iter()
            .all(|value| value != "friction")
    );
}

#[test]
fn collection_parameter_shapes_match_tool_input_normalization() {
    let string_list = tool_parameter_schema("string_list");
    let string_shapes = string_list["anyOf"].as_array().expect("string list union");
    assert!(string_shapes.iter().any(|shape| shape["type"] == "string"));
    assert!(string_shapes.iter().any(|shape| shape["type"] == "array"));

    let object_list = tool_parameter_schema("object_list");
    let object_shapes = object_list["anyOf"].as_array().expect("object list union");
    assert!(object_shapes.iter().any(|shape| shape["type"] == "string"));
    assert!(object_shapes.iter().any(|shape| shape["type"] == "array"));
}

fn json_values(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String((*value).to_string()))
            .collect(),
    )
}
