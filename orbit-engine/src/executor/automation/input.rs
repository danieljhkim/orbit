use std::path::PathBuf;

use orbit_types::{OrbitError, TaskType};
use serde_json::Value;

pub(super) fn required_input_string<'a>(
    input: &'a Value,
    key: &str,
) -> Result<&'a str, OrbitError> {
    input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing required input.{key}")))
}

pub(super) fn input_string_field(input: &Value, key: &str) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn input_string_array_field(
    input: &Value,
    key: &str,
) -> Result<Vec<String>, OrbitError> {
    let Some(values) = input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "input.{key}[{index}] must be a non-empty string"
                    ))
                })
        })
        .collect()
}

pub(super) fn input_workspace_path(input: &Value) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get("workspace_path"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn input_repo_root(input: &Value) -> Result<String, OrbitError> {
    input_string_field(input, "repo_root")
        .or_else(|| input_workspace_path(input))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.repo_root".to_string()))
}

pub(super) fn canonicalize_existing_dir(
    raw: &str,
    field_name: &str,
) -> Result<PathBuf, OrbitError> {
    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "{field_name} does not exist: {raw}"
        )));
    }
    if !path.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "{field_name} is not a directory: {raw}"
        )));
    }
    path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "failed to canonicalize {field_name} '{raw}': {error}"
        ))
    })
}

pub(super) fn task_commit_message(
    task_type: &TaskType,
    title: &str,
    task_id: &str,
    body: &str,
) -> String {
    let prefix = match task_type {
        TaskType::Task | TaskType::Feature => "feat",
        TaskType::Issue => "fix",
        TaskType::Chore => "chore",
        TaskType::Refactor => "refactor",
    };
    let summary = title.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{prefix}: {summary} [{task_id}]\n\n{body}")
}

pub(super) fn json_number_to_string(value: &Value) -> Option<String> {
    value
        .as_i64()
        .map(|number| number.to_string())
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}
