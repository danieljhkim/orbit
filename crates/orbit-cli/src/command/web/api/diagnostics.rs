//! `/diagnostics/{metrics,errors,friction}` aggregation endpoints.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};
use chrono::DateTime;
use orbit_common::utility::blob_store::BlobStore;
use orbit_common::utility::redaction::redact_all;
use orbit_core::{InvocationQuery, InvocationRecord, OrbitRuntime};
use serde_json::{Value, json};

use super::{
    DiagnosticsQuery, HISTORY_DEFAULT_LIMIT, V2_LOOP_FILE_SCAN_CAP, bounded_limit,
    current_year_month_utc, map_runtime_error, month_bounds_utc, server_error, v2_loop_dir,
    validate_year_month,
};
use crate::command::log::format::{
    Filters as LogFilters, read_recent_rendered_events, resolve_log_path,
};

pub(super) async fn list_diagnostics_metrics(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let month = q.month.unwrap_or_else(current_year_month_utc);
    if let Err(e) = validate_year_month(&month) {
        return map_runtime_error(e);
    }
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.read_metrics_entries_limited(&month, limit) {
        Ok(mut entries) => {
            entries.sort_by_key(|entry| std::cmp::Reverse(entry.ts));
            entries.truncate(limit);
            let value = if entries.is_empty() {
                match diagnostics_metrics_from_invocations(&runtime, &month, limit) {
                    Ok(rows) => Value::Array(rows),
                    Err(e) => return map_runtime_error(e),
                }
            } else {
                match serde_json::to_value(&entries) {
                    Ok(value) => value,
                    Err(e) => return server_error(orbit_core::OrbitError::Store(e.to_string())),
                }
            };
            Json(value).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

fn diagnostics_metrics_from_invocations(
    runtime: &OrbitRuntime,
    month: &str,
    limit: usize,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let (since, until) = month_bounds_utc(month)?;
    let records = runtime.invocation_records(InvocationQuery {
        since: Some(since),
        until: Some(until),
        limit,
        ..InvocationQuery::default()
    })?;

    Ok(diagnostics_metrics_values(records))
}

fn diagnostics_metrics_values(records: Vec<InvocationRecord>) -> Vec<Value> {
    records
        .into_iter()
        .map(|record| {
            json!({
                "ts": record.ts.to_rfc3339(),
                "job_run": record.job_run_id,
                "step": record.activity_id,
                "task_id": record.task_ids.first().cloned(),
                "actor_identity": actor_label(&record.agent, record.model.as_deref()),
                "tool_invocations": record.tool_call_count,
                "token_usage": record.total_tokens,
                "step_duration_ms": record.duration_ms,
                "retry_count": 0,
            })
        })
        .collect()
}

fn actor_label(agent: &str, model: Option<&str>) -> String {
    match model.filter(|model| !model.is_empty()) {
        Some(model) if !agent.is_empty() => format!("{agent} / {model}"),
        Some(model) => model.to_string(),
        None => agent.to_string(),
    }
}

fn diagnostics_friction_from_v2_audit(
    runtime: &OrbitRuntime,
    month: &str,
    limit: usize,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    validate_year_month(month)?;
    if limit == 0 {
        return Ok(Vec::new());
    }

    let audit_dir = v2_loop_dir(runtime);
    if !audit_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = std::fs::read_dir(&audit_dir)
        .map_err(|e| orbit_core::OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort();
    if files.len() > V2_LOOP_FILE_SCAN_CAP {
        files = files.split_off(files.len() - V2_LOOP_FILE_SCAN_CAP);
    }

    let blob_store = BlobStore::new(
        runtime
            .data_root()
            .join("state")
            .join("audit")
            .join("blobs"),
    );
    let mut rows = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| orbit_core::OrbitError::Io(format!("read {}: {e}", path.display())))?;
        let mut events = Vec::new();
        let mut by_id = HashMap::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(event_id) = value.get("event_id").and_then(Value::as_str) {
                by_id.insert(event_id.to_string(), value.clone());
            }
            events.push(value);
        }

        for event in events {
            if let Some(row) = diagnostics_friction_row(&blob_store, &by_id, &event, month) {
                rows.push(row);
            }
        }
    }

    rows.sort_by(|a, b| {
        let left = a.get("ts").and_then(Value::as_str).unwrap_or("");
        let right = b.get("ts").and_then(Value::as_str).unwrap_or("");
        right.cmp(left)
    });
    rows.truncate(limit);
    Ok(rows)
}

fn diagnostics_friction_row(
    blob_store: &BlobStore,
    events_by_id: &HashMap<String, Value>,
    event: &Value,
    month: &str,
) -> Option<Value> {
    let ts = event.get("ts").and_then(Value::as_str)?;
    if !ts.starts_with(month) {
        return None;
    }

    let body_kind = event.get("body_kind").and_then(Value::as_str).unwrap_or("");
    match body_kind {
        "cli_invocation_finished" => {
            let exit_code = event.get("exit_code").and_then(Value::as_i64);
            let timed_out = event
                .get("timed_out")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if exit_code == Some(0) && !timed_out {
                return None;
            }
            Some(json!({
                "ts": ts,
                "job_run": event.get("run_id").and_then(Value::as_str).unwrap_or(""),
                "step": enclosing_step_id_for_event(event, events_by_id).unwrap_or_default(),
                "task_id": null,
                "command": event.get("provider").and_then(Value::as_str).unwrap_or("cli"),
                "input": "",
                "exit_code": exit_code,
                "stderr": event
                    .get("stderr_blob_ref")
                    .and_then(Value::as_str)
                    .map(|blob_ref| read_blob_text_best_effort(blob_store, blob_ref))
                    .unwrap_or_default(),
                "actor_identity": event.get("agent_identity").cloned().unwrap_or(Value::Null),
            }))
        }
        "step_finished" => {
            let outcome = event
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("success");
            if matches!(outcome, "success" | "skipped") {
                return None;
            }
            let step = event.get("step_id").and_then(Value::as_str).unwrap_or("");
            Some(json!({
                "ts": ts,
                "job_run": event.get("run_id").and_then(Value::as_str).unwrap_or(""),
                "step": step,
                "task_id": null,
                "command": step,
                "input": "",
                "exit_code": null,
                "stderr": event
                    .get("error_message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("step finished with outcome '{outcome}'")),
                "actor_identity": event.get("agent_identity").cloned().unwrap_or(Value::Null),
            }))
        }
        "step_denied" | "tool_denied" | "fs_call_denied" => {
            let step = enclosing_step_id_for_event(event, events_by_id).unwrap_or_default();
            Some(json!({
                "ts": ts,
                "job_run": event.get("run_id").and_then(Value::as_str).unwrap_or(""),
                "step": step,
                "task_id": null,
                "command": event
                    .get("tool_name")
                    .or_else(|| event.get("op"))
                    .and_then(Value::as_str)
                    .unwrap_or(body_kind),
                "input": "",
                "exit_code": null,
                "stderr": event
                    .get("reason")
                    .or_else(|| event.get("matched_rule"))
                    .and_then(Value::as_str)
                    .unwrap_or(body_kind),
                "actor_identity": event.get("agent_identity").cloned().unwrap_or(Value::Null),
            }))
        }
        _ => None,
    }
}

fn enclosing_step_id_for_event(
    event: &Value,
    events_by_id: &HashMap<String, Value>,
) -> Option<String> {
    if let Some(step_id) = event.get("step_id").and_then(Value::as_str) {
        return Some(step_id.to_string());
    }

    let mut parent_id = event
        .get("parent_event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut seen = HashSet::new();
    while let Some(id) = parent_id {
        if !seen.insert(id.clone()) {
            return None;
        }
        let parent = events_by_id.get(&id)?;
        if parent.get("body_kind").and_then(Value::as_str) == Some("step_started") {
            return parent
                .get("step_id")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        parent_id = parent
            .get("parent_event_id")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    None
}

fn read_blob_text_best_effort(blob_store: &BlobStore, blob_ref: &str) -> String {
    if blob_ref.len() < 2 || blob_ref.starts_with("error:") {
        return String::new();
    }
    blob_store
        .read(blob_ref)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

pub(super) async fn list_diagnostics_errors(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match diagnostics_errors(&runtime, limit) {
        Ok(rows) => Json(Value::Array(rows)).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

fn diagnostics_errors(
    runtime: &OrbitRuntime,
    limit: usize,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut rows = global_error_rows(limit)?;
    rows.extend(agent_stderr_error_rows(runtime, limit)?);
    rows.sort_by(|a, b| {
        let left = a.get("ts").and_then(Value::as_str).unwrap_or("");
        let right = b.get("ts").and_then(Value::as_str).unwrap_or("");
        right.cmp(left)
    });
    rows.truncate(limit);
    Ok(rows)
}

fn global_error_rows(limit: usize) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let path = resolve_log_path(None)?;
    global_error_rows_from_path(&path, limit)
}

fn global_error_rows_from_path(
    path: &std::path::Path,
    limit: usize,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let filters = LogFilters::from_query_parts(None, Some("error".to_string()), None)?;
    let events = read_recent_rendered_events(path, &filters, limit)
        .map_err(|e| orbit_core::OrbitError::Io(format!("read log {}: {e}", path.display())))?;
    Ok(events
        .into_iter()
        .map(|event| {
            json!({
                "ts": event.ts,
                "source": "process",
                "message": strip_htmlish(&event.message_html),
                "event_id": null,
                "job_run": null,
                "step": null,
                "step_index": null,
                "task_id": null,
                "provider": null,
                "blob_ref": null,
                "target": event.source,
            })
        })
        .collect())
}

fn agent_stderr_error_rows(
    runtime: &OrbitRuntime,
    limit: usize,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let audit_dir = v2_loop_dir(runtime);
    if !audit_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = std::fs::read_dir(&audit_dir)
        .map_err(|e| orbit_core::OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort();
    if files.len() > V2_LOOP_FILE_SCAN_CAP {
        files = files.split_off(files.len() - V2_LOOP_FILE_SCAN_CAP);
    }
    files.reverse();

    let blob_store = BlobStore::new(
        runtime
            .data_root()
            .join("state")
            .join("audit")
            .join("blobs"),
    );
    let mut rows = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| orbit_core::OrbitError::Io(format!("read {}: {e}", path.display())))?;
        let mut events = Vec::new();
        let mut by_id = HashMap::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(event_id) = value.get("event_id").and_then(Value::as_str) {
                by_id.insert(event_id.to_string(), value.clone());
            }
            events.push(value);
        }
        let step_index_by_id = step_index_by_id(&events);
        for event in events {
            if event.get("body_kind").and_then(Value::as_str) != Some("cli_invocation_finished") {
                continue;
            }
            let Some(blob_ref) = event.get("stderr_blob_ref").and_then(Value::as_str) else {
                continue;
            };
            let stderr = read_blob_text_best_effort(&blob_store, blob_ref);
            let fallback_ts = event
                .get("ts")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let step = enclosing_step_id_for_event(&event, &by_id);
            let step_index = step
                .as_ref()
                .and_then(|step| step_index_by_id.get(step).copied());
            for parsed in parse_structured_error_lines(&stderr, &fallback_ts) {
                rows.push(json!({
                    "ts": parsed.ts,
                    "source": "agent-stderr",
                    "message": redact_all(&parsed.message),
                    "job_run": event.get("run_id").and_then(Value::as_str),
                    "step": step,
                    "step_index": step_index,
                    "task_id": event.get("task_id").and_then(Value::as_str),
                    "provider": event.get("provider").and_then(Value::as_str),
                    "blob_ref": blob_ref,
                    "event_id": event.get("event_id").and_then(Value::as_str),
                    "target": parsed.target,
                }));
                if rows.len() >= limit.saturating_mul(2) {
                    return Ok(rows);
                }
            }
        }
    }
    Ok(rows)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedErrorLine {
    ts: String,
    target: String,
    message: String,
}

fn parse_structured_error_lines(stderr: &str, fallback_ts: &str) -> Vec<ParsedErrorLine> {
    stderr
        .lines()
        .filter_map(|line| parse_structured_error_line(line, fallback_ts))
        .collect()
}

fn parse_structured_error_line(line: &str, fallback_ts: &str) -> Option<ParsedErrorLine> {
    let trimmed = line.trim();
    let (ts, rest) = if let Some((head, tail)) = trimmed.split_once(" ERROR ") {
        if DateTime::parse_from_rfc3339(head).is_ok() {
            (head.to_string(), tail)
        } else {
            (fallback_ts.to_string(), trimmed.strip_prefix("ERROR ")?)
        }
    } else {
        (fallback_ts.to_string(), trimmed.strip_prefix("ERROR ")?)
    };
    let (target, message) = rest.rsplit_once(": ")?;
    let target = target.trim();
    let message = message.trim();
    if target.is_empty() || message.is_empty() {
        return None;
    }
    Some(ParsedErrorLine {
        ts,
        target: target.to_string(),
        message: message.to_string(),
    })
}

fn step_index_by_id(events: &[Value]) -> HashMap<String, u32> {
    let mut result = HashMap::new();
    for event in events {
        if event.get("body_kind").and_then(Value::as_str) != Some("step_started") {
            continue;
        }
        let Some(step_id) = event.get("step_id").and_then(Value::as_str) else {
            continue;
        };
        let index = result.len() as u32;
        result.entry(step_id.to_string()).or_insert(index);
    }
    result
}

fn strip_htmlish(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}

pub(super) async fn list_diagnostics_friction(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let month = q.month.unwrap_or_else(current_year_month_utc);
    if let Err(e) = validate_year_month(&month) {
        return map_runtime_error(e);
    }
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.read_friction_entries_limited(&month, limit) {
        Ok(mut entries) => {
            entries.sort_by_key(|entry| std::cmp::Reverse(entry.ts));
            entries.truncate(limit);
            let value = if entries.is_empty() {
                match diagnostics_friction_from_v2_audit(&runtime, &month, limit) {
                    Ok(rows) => Value::Array(rows),
                    Err(e) => return map_runtime_error(e),
                }
            } else {
                match serde_json::to_value(&entries) {
                    Ok(value) => value,
                    Err(e) => return server_error(orbit_core::OrbitError::Store(e.to_string())),
                }
            };
            Json(value).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
