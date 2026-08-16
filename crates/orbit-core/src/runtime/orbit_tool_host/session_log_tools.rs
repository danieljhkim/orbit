use chrono::{DateTime, Utc};
use orbit_common::OrbitError;
use orbit_store::{SessionLogAppendParams, SessionLogFilter, SessionLogKind, SessionLogStore};
use serde_json::{Value, json};

use crate::OrbitRuntime;

pub(super) fn append_entry(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let kind = parse_kind(&input)?;
    let body = input
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("body is required".to_string()))?
        .to_string();
    let entry = session_log_store(runtime).append(SessionLogAppendParams {
        kind,
        body,
        related_task_ids: parse_id_list(&input, "related_task_ids")?,
        related_run_ids: parse_id_list(&input, "related_run_ids")?,
    })?;
    Ok(json!(entry))
}

pub(super) fn list_entries(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let unresolved_only = input
        .get("unresolved_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entries = session_log_store(runtime).list(SessionLogFilter {
        kind: parse_optional_kind(&input)?,
        unresolved_only,
        since: parse_since(&input)?,
    })?;
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
    let entry = session_log_store(runtime).resolve(id)?;
    Ok(json!(entry))
}

fn session_log_store(runtime: &OrbitRuntime) -> SessionLogStore {
    SessionLogStore::new(runtime.paths().orbit_dir.clone())
}

fn parse_kind(input: &Value) -> Result<SessionLogKind, OrbitError> {
    let raw = input
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("kind is required".to_string()))?;
    parse_kind_name(raw)
}

fn parse_optional_kind(input: &Value) -> Result<Option<SessionLogKind>, OrbitError> {
    match input.get("kind") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => parse_kind_name(raw).map(Some),
        Some(_) => Err(OrbitError::InvalidInput(
            "kind must be a string".to_string(),
        )),
    }
}

fn parse_kind_name(raw: &str) -> Result<SessionLogKind, OrbitError> {
    match raw.trim() {
        "status" => Ok(SessionLogKind::Status),
        "note" => Ok(SessionLogKind::Note),
        "check_later" => Ok(SessionLogKind::CheckLater),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown session-log kind `{other}` (expected status, note, or check_later)"
        ))),
    }
}

fn parse_id_list(input: &Value, field: &str) -> Result<Vec<String>, OrbitError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(format!("{field} must be an array of strings"))
                    })
            })
            .collect(),
        Some(_) => Err(OrbitError::InvalidInput(format!(
            "{field} must be an array of strings"
        ))),
    }
}

fn parse_since(input: &Value) -> Result<Option<DateTime<Utc>>, OrbitError> {
    match input.get("since") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => DateTime::parse_from_rfc3339(raw)
            .map(|date_time| Some(date_time.with_timezone(&Utc)))
            .map_err(|error| OrbitError::InvalidInput(format!("since must be RFC3339: {error}"))),
        Some(_) => Err(OrbitError::InvalidInput(
            "since must be an RFC3339 string".to_string(),
        )),
    }
}
