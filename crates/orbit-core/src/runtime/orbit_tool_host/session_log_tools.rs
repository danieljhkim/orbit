use orbit_common::types::OrbitError;
use serde_json::{json, Value};

use crate::runtime::session_log::{
    append, list, parse_id_list, parse_kind, parse_optional_kind, parse_since, resolve,
};
use crate::OrbitRuntime;

pub(super) fn append_entry(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let kind = parse_kind(&input)?;
    let body = input
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("body is required".to_string()))?
        .to_string();
    let entry = append(
        &runtime.paths().orbit_dir,
        kind,
        body,
        parse_id_list(&input, "related_task_ids")?,
        parse_id_list(&input, "related_run_ids")?,
    )?;
    Ok(json!(entry))
}

pub(super) fn list_entries(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let unresolved_only = input
        .get("unresolved_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entries = list(
        &runtime.paths().orbit_dir,
        parse_optional_kind(&input)?,
        unresolved_only,
        parse_since(&input)?,
    )?;
    Ok(json!({
        "entries": entries,
        "count": entries.len(),
    }))
}

pub(super) fn resolve_entry(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = input
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("id is required".to_string()))?;
    let entry = resolve(&runtime.paths().orbit_dir, id)?;
    Ok(json!(entry))
}
