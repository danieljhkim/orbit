//! Helpers shared by more than one `orbit adr` subcommand body.

use std::fs;
use std::path::PathBuf;

use orbit_core::OrbitError;
use serde_json::Value;

/// Resolve the mutually exclusive `--body` / `--body-file` pair.
///
/// `required` distinguishes the authoring verbs (`add`, `restore`), where a
/// body is mandatory, from `update`, where omitting both leaves the stored
/// body untouched.
pub(super) fn resolve_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
    required: bool,
) -> Result<Option<String>, OrbitError> {
    match (body, body_file) {
        (Some(_), Some(_)) => Err(OrbitError::InvalidInput(
            "specify exactly one of `--body` and `--body-file`".to_string(),
        )),
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) => fs::read_to_string(&path)
            .map(Some)
            .map_err(|e| OrbitError::Io(format!("read body file {}: {e}", path.display()))),
        (None, None) if required => Err(OrbitError::InvalidInput(
            "specify exactly one of `--body` and `--body-file`".to_string(),
        )),
        (None, None) => Ok(None),
    }
}

/// Project a repeatable list flag onto the tool's replacement-list semantics.
///
/// `orbit.adr.update` treats an absent key as "leave unchanged" and an empty
/// array as "clear", which a `Vec<String>` alone cannot express. Following
/// `orbit learning update`, passing the flag once with an empty string clears
/// the field; omitting it entirely preserves the stored value.
pub(super) fn replacement_list(values: Vec<String>) -> Option<Vec<String>> {
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect(),
    )
}

/// The canonical ID a mutating `orbit.adr.*` tool response carries, for the
/// human (non-`--json`) output mode.
pub(super) fn response_id(value: &Value) -> &str {
    value["id"].as_str().unwrap_or_default()
}

/// A string field of an ADR record, or `-` when it is absent or null.
///
/// The records come from the tool as `Value`, not a typed struct, so the list
/// view reads them defensively rather than unwrapping.
pub(super) fn field_str(record: &Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}

/// A string-array field, comma-joined for the table cell.
pub(super) fn field_list(record: &Value, key: &str) -> String {
    let Some(values) = record.get(key).and_then(Value::as_array) else {
        return "-".to_string();
    };
    if values.is_empty() {
        return "-".to_string();
    }
    values
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
