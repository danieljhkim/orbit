//! Policy and tool denial aggregation across the v2 audit envelope and the
//! SQLite audit-events table.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use orbit_core::{AuditEventStatus, OrbitRuntime};
use serde_json::{Value, json};

use super::{
    DEFAULT_SUMMARY_WINDOW, DenialsQuery, V2_LOOP_FILE_SCAN_CAP, bad_request, map_runtime_error,
    server_error, v2_loop_dir,
};
use crate::p::parse_since;

const SQLITE_DENIAL_SCAN_LIMIT: usize = 1000;
pub(super) const SQLITE_FS_BOUNDARY_PROFILE: &str = "workspace-boundary";
const SQLITE_TOOL_DENIAL_PROFILE: &str = "tool";

/// Internal denial event extracted from the v2 envelope JSONL.
#[derive(Debug, Clone)]
pub(super) struct DenialRow {
    kind: &'static str,
    profile: String,
    target: String,
    job_run_id: Option<String>,
    execution_id: Option<String>,
    agent: String,
    timestamp: Option<DateTime<Utc>>,
    diagnostics: DenialDiagnostics,
}

#[derive(Debug, Clone)]
struct DenialDiagnostics {
    denial_kind: String,
    cause: String,
    actor: Option<String>,
    requested_task_ids: Vec<String>,
    requested_files: Vec<String>,
    conflicts: Vec<Value>,
}

impl Default for DenialDiagnostics {
    fn default() -> Self {
        Self {
            denial_kind: "unknown".to_string(),
            cause: "unknown".to_string(),
            actor: None,
            requested_task_ids: Vec::new(),
            requested_files: Vec::new(),
            conflicts: Vec::new(),
        }
    }
}

impl DenialRow {
    #[cfg(test)]
    pub(super) fn target(&self) -> &str {
        &self.target
    }
}

/// Walks `state/audit/v2_loop/*.jsonl` and returns FsCallDenied / ToolDenied
/// rows matching the supplied filters. Bounded by `V2_LOOP_FILE_SCAN_CAP` files.
pub(super) fn scan_v2_loop_denials(
    runtime: &OrbitRuntime,
    since: Option<DateTime<Utc>>,
    profile_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Vec<DenialRow>, orbit_core::OrbitError> {
    let dir = v2_loop_dir(runtime);
    let mut out: Vec<DenialRow> = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(orbit_core::OrbitError::Io(format!(
                "read {}: {e}",
                dir.display()
            )));
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();
    if paths.len() > V2_LOOP_FILE_SCAN_CAP {
        let drop = paths.len() - V2_LOOP_FILE_SCAN_CAP;
        paths.drain(0..drop);
    }

    for path in paths {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let kind_raw = value.get("body_kind").and_then(Value::as_str).unwrap_or("");
            let kind = match kind_raw {
                "fs_call_denied" => "fs",
                "tool_denied" => "tool",
                _ => continue,
            };
            let ts = value
                .get("ts")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));
            if let Some(since) = since
                && let Some(ts) = ts
                && ts < since
            {
                continue;
            }
            let run_id = value
                .get("run_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            let agent = value
                .get("agent_identity")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let (profile, target) = match kind {
                "fs" => (
                    value
                        .get("profile")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => (
                    "tool".to_string(),
                    value
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
            };
            if let Some(want) = profile_filter
                && !want.is_empty()
                && profile != want
            {
                continue;
            }
            if let Some(want) = agent_filter
                && !want.is_empty()
                && agent != want
            {
                continue;
            }
            out.push(DenialRow {
                kind,
                diagnostics: v2_denial_diagnostics(kind, &profile, &target),
                profile,
                target,
                job_run_id: run_id,
                execution_id: None,
                agent,
                timestamp: ts,
            });
        }
    }
    Ok(out)
}

pub(super) fn collect_denial_rows(
    runtime: &OrbitRuntime,
    since: Option<DateTime<Utc>>,
    profile_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Vec<DenialRow>, orbit_core::OrbitError> {
    let mut rows = scan_v2_loop_denials(runtime, since, profile_filter, agent_filter)?;
    rows.extend(scan_sqlite_denials(
        runtime,
        since,
        profile_filter,
        agent_filter,
    )?);
    Ok(rows)
}

fn scan_sqlite_denials(
    runtime: &OrbitRuntime,
    since: Option<DateTime<Utc>>,
    profile_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Vec<DenialRow>, orbit_core::OrbitError> {
    let events = runtime.list_audit_events(
        since,
        None,
        Some(AuditEventStatus::Denied),
        agent_filter.map(ToOwned::to_owned),
        SQLITE_DENIAL_SCAN_LIMIT,
    )?;

    let rows = events
        .into_iter()
        .map(|event| sqlite_denial_row(&event))
        .filter(|row| {
            profile_filter
                .map(|want| want.is_empty() || row.profile == want)
                .unwrap_or(true)
        })
        .collect();
    Ok(rows)
}

fn sqlite_denial_row(event: &orbit_core::AuditEvent) -> DenialRow {
    let kind = sqlite_denial_kind(event);
    let arguments_json = parse_arguments_json(event.arguments_json.as_deref());
    let diagnostics = sqlite_denial_diagnostics(event, kind, arguments_json.as_ref());
    DenialRow {
        kind,
        profile: sqlite_denial_profile(event, kind, arguments_json.as_ref()),
        target: sqlite_denial_target(event, kind, arguments_json.as_ref()),
        job_run_id: event.job_run_id.clone().filter(|value| !value.is_empty()),
        execution_id: Some(event.execution_id.clone()).filter(|value| !value.is_empty()),
        agent: event.role.clone(),
        timestamp: Some(event.timestamp),
        diagnostics,
    }
}

fn sqlite_denial_kind(event: &orbit_core::AuditEvent) -> &'static str {
    let tool_name = event.tool_name.as_deref().unwrap_or("");
    if tool_name.starts_with("fs.") {
        "fs"
    } else {
        "tool"
    }
}

fn sqlite_denial_profile(
    event: &orbit_core::AuditEvent,
    kind: &str,
    arguments_json: Option<&Value>,
) -> String {
    if kind != "fs" {
        return SQLITE_TOOL_DENIAL_PROFILE.to_string();
    }
    if let Some(profile) = arguments_json_profile(arguments_json) {
        return profile;
    }
    if let Some(profile) = extract_fs_profile_from_policy_message(event.error_message.as_deref()) {
        return profile;
    }
    SQLITE_FS_BOUNDARY_PROFILE.to_string()
}

fn sqlite_denial_target(
    event: &orbit_core::AuditEvent,
    kind: &str,
    arguments_json: Option<&Value>,
) -> String {
    if kind == "fs"
        && let Some(path) = extract_fs_path_from_policy_message(event.error_message.as_deref())
    {
        return path;
    }
    if is_task_lock_reserve_denial(event) {
        let requested_files = string_array_field(arguments_json, "files");
        if let Some(file) = requested_files.first() {
            return file.clone();
        }
        let task_ids = string_array_field(arguments_json, "task_ids");
        if !task_ids.is_empty() {
            return task_ids.join(", ");
        }
    }
    event
        .target_id
        .clone()
        .or_else(|| event.tool_name.clone())
        .or_else(|| event.subcommand.clone())
        .unwrap_or_else(|| event.command.clone())
}

fn parse_arguments_json(raw: Option<&str>) -> Option<Value> {
    serde_json::from_str(raw?).ok()
}

fn arguments_json_profile(value: Option<&Value>) -> Option<String> {
    const KEYS: &[&str] = &["fsProfile", "fs_profile", "profile"];
    let obj = value?.as_object()?;
    for key in KEYS {
        if let Some(Value::String(found)) = obj.get(*key)
            && !found.is_empty()
        {
            return Some(found.clone());
        }
    }
    None
}

fn v2_denial_diagnostics(kind: &str, profile: &str, target: &str) -> DenialDiagnostics {
    match kind {
        "fs" => DenialDiagnostics {
            denial_kind: "fs_policy".to_string(),
            cause: if profile.is_empty() {
                "fs_denied".to_string()
            } else {
                profile.to_string()
            },
            requested_files: if target.is_empty() {
                Vec::new()
            } else {
                vec![target.to_string()]
            },
            ..DenialDiagnostics::default()
        },
        _ => DenialDiagnostics {
            denial_kind: "tool_policy".to_string(),
            cause: if target.is_empty() {
                "tool_denied".to_string()
            } else {
                target.to_string()
            },
            ..DenialDiagnostics::default()
        },
    }
}

fn sqlite_denial_diagnostics(
    event: &orbit_core::AuditEvent,
    kind: &str,
    arguments_json: Option<&Value>,
) -> DenialDiagnostics {
    if is_task_lock_reserve_denial(event) {
        let conflicts = value_array_field(arguments_json, "conflicts");
        return DenialDiagnostics {
            denial_kind: "task_lock_reserve".to_string(),
            cause: if conflicts.is_empty() {
                "task_lock_denied".to_string()
            } else {
                "task_lock_conflict".to_string()
            },
            actor: string_field(arguments_json, "actor"),
            requested_task_ids: string_array_field(arguments_json, "task_ids"),
            requested_files: string_array_field(arguments_json, "files"),
            conflicts,
        };
    }

    if kind == "fs" {
        let profile = sqlite_denial_profile(event, kind, arguments_json);
        return DenialDiagnostics {
            denial_kind: "fs_policy".to_string(),
            cause: profile,
            requested_files: extract_fs_path_from_policy_message(event.error_message.as_deref())
                .into_iter()
                .collect(),
            ..DenialDiagnostics::default()
        };
    }

    DenialDiagnostics {
        denial_kind: "tool_policy".to_string(),
        cause: event
            .tool_name
            .clone()
            .or_else(|| event.subcommand.clone())
            .unwrap_or_else(|| "tool_denied".to_string()),
        ..DenialDiagnostics::default()
    }
}

fn is_task_lock_reserve_denial(event: &orbit_core::AuditEvent) -> bool {
    event.tool_name.as_deref() == Some("orbit.task.locks.reserve")
        || event.command == "task.locks.reserve.denied"
}

fn string_field(value: Option<&Value>, key: &str) -> Option<String> {
    value?
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_field(value: Option<&Value>, key: &str) -> Vec<String> {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn value_array_field(value: Option<&Value>, key: &str) -> Vec<Value> {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn extract_fs_profile_from_policy_message(message: Option<&str>) -> Option<String> {
    extract_between(message?, "under fsProfile `", "`")
}

fn extract_fs_path_from_policy_message(message: Option<&str>) -> Option<String> {
    let message = message?;
    if let Some(path) = extract_denied_for_path(message) {
        return Some(path);
    }
    extract_after_prefix(message, "path is outside workspace: ")
}

fn extract_denied_for_path(message: &str) -> Option<String> {
    let marker = " denied for `";
    let marker_idx = message.find(marker)?;
    let prefix = &message[..marker_idx];
    if !prefix.ends_with("fs.read")
        && !prefix.ends_with("fs.modify")
        && !prefix.ends_with("fs.delete")
    {
        return None;
    }
    let rest = &message[marker_idx + marker.len()..];
    let end = rest.find('`')?;
    let path = rest[..end].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn extract_between(message: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = message.find(start)? + start.len();
    let rest = &message[start_idx..];
    let end_idx = rest.find(end)?;
    let value = rest[..end_idx].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn extract_after_prefix(message: &str, prefix: &str) -> Option<String> {
    let start_idx = message.find(prefix)? + prefix.len();
    let value = message[start_idx..]
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.');
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Top-N tool denials for the audit-summary side panel. Targets `kind == "tool"`
/// rows (so `target` is a tool name); fs rows are excluded because their `target`
/// is a path and would clutter the per-tool list.
pub(super) fn denials_by_tool_summary(rows: &[DenialRow], limit: usize) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        if row.kind != "tool" {
            continue;
        }
        if row.target.is_empty() {
            continue;
        }
        *counts.entry(row.target.clone()).or_insert(0) += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Value::Array(
        out.into_iter()
            .take(limit)
            .map(|(tool, count)| json!({"tool": tool, "count": count}))
            .collect(),
    )
}

/// Top-N denial causes (fs profile, tool name, lock conflict, etc.) across all
/// denial kinds for the audit-summary side panel.
pub(super) fn denials_by_reason_summary(rows: &[DenialRow], limit: usize) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let cause = row.diagnostics.cause.clone();
        if cause.is_empty() {
            continue;
        }
        *counts.entry(cause).or_insert(0) += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Value::Array(
        out.into_iter()
            .take(limit)
            .map(|(reason, count)| json!({"reason": reason, "count": count}))
            .collect(),
    )
}

pub(super) async fn list_denials(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DenialsQuery>,
) -> Response {
    let raw_since = q.since.as_deref().unwrap_or(DEFAULT_SUMMARY_WINDOW);
    let since = match parse_since(raw_since) {
        Ok(ts) => Some(ts),
        Err(e) => return map_runtime_error(e),
    };
    let kind = q
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    if let Some(ref k) = kind
        && k != "fs"
        && k != "tool"
    {
        return bad_request(format!("kind must be 'fs', 'tool', or omitted; got '{k}'"));
    }
    let profile_filter = q
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent_filter = q.agent.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let rows = match collect_denial_rows(&runtime, since, profile_filter, agent_filter) {
        Ok(rows) => rows,
        Err(e) => return server_error(e),
    };

    Json(denials_payload(&rows, kind.as_deref(), since)).into_response()
}

fn denials_payload(rows: &[DenialRow], kind: Option<&str>, since: Option<DateTime<Utc>>) -> Value {
    let filtered = filter_denial_rows(rows, kind);

    let by_profile = aggregate_by(&filtered, |r| r.profile.clone());
    let by_target = aggregate_by(&filtered, |r| r.target.clone());
    let by_run = aggregate_by_optional(&filtered, |r| r.job_run_id.clone());
    let by_execution = aggregate_by_optional(&filtered, |r| {
        if r.job_run_id.is_none() {
            r.execution_id.clone()
        } else {
            None
        }
    });
    let by_agent = aggregate_by(&filtered, |r| r.agent.clone());
    let top_causes = top_causes_to_value(&filtered);
    let recent_denials = recent_denials_to_value(&filtered, 12);

    json!({
        "by_profile": rows_to_value(&by_profile, "name"),
        "by_target": rows_to_value(&by_target, "name"),
        "by_run": rows_to_value(&by_run, "run_id"),
        "by_execution": rows_to_value(&by_execution, "execution_id"),
        "by_agent": rows_to_value(&by_agent, "agent"),
        "top_causes": top_causes,
        "recent_denials": recent_denials,
        "total": filtered.len(),
        "kind": kind,
        "since": since.map(|s| s.to_rfc3339()),
    })
}

fn filter_denial_rows<'a>(rows: &'a [DenialRow], kind: Option<&str>) -> Vec<&'a DenialRow> {
    rows.iter()
        .filter(|r| match kind {
            None => true,
            Some(k) => r.kind == k,
        })
        .collect()
}

fn aggregate_by<F>(rows: &[&DenialRow], key: F) -> Vec<(String, i64)>
where
    F: Fn(&DenialRow) -> String,
{
    aggregate_by_optional(rows, |row| Some(key(row)))
}

fn aggregate_by_optional<F>(rows: &[&DenialRow], key: F) -> Vec<(String, i64)>
where
    F: Fn(&DenialRow) -> Option<String>,
{
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let Some(k) = key(row) else {
            continue;
        };
        if k.is_empty() {
            continue;
        }
        *counts.entry(k).or_insert(0) += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn rows_to_value(rows: &[(String, i64)], key_label: &str) -> Value {
    Value::Array(
        rows.iter()
            .map(|(name, count)| json!({ key_label: name, "count": count }))
            .collect(),
    )
}

#[derive(Debug, Default)]
struct CauseAggregate {
    count: i64,
    latest: Option<DateTime<Utc>>,
    targets: BTreeMap<String, i64>,
}

fn top_causes_to_value(rows: &[&DenialRow]) -> Value {
    let mut by_cause: BTreeMap<String, CauseAggregate> = BTreeMap::new();
    for row in rows {
        let cause = row.diagnostics.cause.clone();
        if cause.is_empty() {
            continue;
        }
        let entry = by_cause.entry(cause).or_default();
        entry.count += 1;
        if row.timestamp > entry.latest {
            entry.latest = row.timestamp;
        }
        if !row.target.is_empty() {
            *entry.targets.entry(row.target.clone()).or_insert(0) += 1;
        }
    }

    let mut out: Vec<_> = by_cause.into_iter().collect();
    out.sort_by(|(left_cause, left), (right_cause, right)| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| right.latest.cmp(&left.latest))
            .then_with(|| left_cause.cmp(right_cause))
    });
    Value::Array(
        out.into_iter()
            .take(8)
            .map(|(cause, aggregate)| {
                let target = aggregate
                    .targets
                    .into_iter()
                    .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
                    .map(|(target, _count)| target);
                json!({
                    "cause": cause,
                    "target": target,
                    "count": aggregate.count,
                    "latest_ts": aggregate.latest.map(|ts| ts.to_rfc3339()),
                })
            })
            .collect(),
    )
}

fn recent_denials_to_value(rows: &[&DenialRow], limit: usize) -> Value {
    let mut rows = rows.to_vec();
    rows.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| left.target.cmp(&right.target))
    });
    Value::Array(
        rows.into_iter()
            .take(limit)
            .map(denial_row_to_value)
            .collect(),
    )
}

fn denial_row_to_value(row: &DenialRow) -> Value {
    let (identity_type, identity_id) = if let Some(job_run_id) = row.job_run_id.as_deref() {
        ("job_run", Some(job_run_id))
    } else if let Some(execution_id) = row.execution_id.as_deref() {
        ("audit_execution", Some(execution_id))
    } else {
        ("none", None)
    };

    json!({
        "kind": row.kind,
        "profile": row.profile.clone(),
        "target": row.target.clone(),
        "run_id": row.job_run_id.clone(),
        "job_run_id": row.job_run_id.clone(),
        "execution_id": row.execution_id.clone(),
        "identity_type": identity_type,
        "identity_id": identity_id,
        "agent": row.agent.clone(),
        "timestamp": row.timestamp.map(|ts| ts.to_rfc3339()),
        "denial_kind": row.diagnostics.denial_kind.clone(),
        "cause": row.diagnostics.cause.clone(),
        "actor": row.diagnostics.actor.clone(),
        "requested_task_ids": row.diagnostics.requested_task_ids.clone(),
        "requested_files": row.diagnostics.requested_files.clone(),
        "conflicts": row.diagnostics.conflicts.clone(),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use orbit_core::OrbitRuntime;

    use super::super::test_support::write_lines;
    use super::*;

    #[test]
    fn denials_payload_combines_v2_and_sqlite_denials() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let audit_dir = runtime.data_root().join("state/audit/v2_loop");
        std::fs::create_dir_all(&audit_dir).expect("create audit dir");
        let now = Utc::now();
        write_lines(
            &audit_dir.join("run-v2-denials.jsonl"),
            &[
                json!({
                    "schemaVersion": 1,
                    "event_type": "fs.call.denied",
                    "event_id": "evt-fs-denied",
                    "ts": now.to_rfc3339(),
                    "run_id": "run-v2-denials",
                    "agent_identity": "codex / gpt-5",
                    "body_kind": "fs_call_denied",
                    "profile": "restricted",
                    "path": "./secret.txt"
                })
                .to_string(),
                json!({
                    "schemaVersion": 1,
                    "event_type": "tool.denied",
                    "event_id": "evt-tool-denied",
                    "ts": now.to_rfc3339(),
                    "run_id": "run-v2-denials",
                    "agent_identity": "codex / gpt-5",
                    "body_kind": "tool_denied",
                    "tool_name": "github.pr.merge"
                })
                .to_string(),
            ],
        );
        runtime
            .record_audit_event(&orbit_core::AuditEventInsertParams {
                execution_id: "exec-sqlite-fs".to_string(),
                command: "tool".to_string(),
                subcommand: Some("fs.read".to_string()),
                tool_name: Some("fs.read".to_string()),
                target_type: Some("tool".to_string()),
                target_id: Some("fs.read".to_string()),
                role: "codex".to_string(),
                status: AuditEventStatus::Denied,
                exit_code: 1,
                duration_ms: 5,
                working_directory: "/workspace".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: Some("path is outside workspace: /usr/bin/false".to_string()),
                host: None,
                pid: 123,
                session_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("record sqlite denial");

        let since = now - Duration::minutes(5);
        let rows = collect_denial_rows(&runtime, Some(since), None, None).expect("collect denials");
        let payload = denials_payload(&rows, None, Some(since));
        assert_eq!(payload["total"], 3);
        assert!(payload["by_target"].to_string().contains("/usr/bin/false"));
        assert!(payload["by_target"].to_string().contains("./secret.txt"));
        assert!(payload["by_target"].to_string().contains("github.pr.merge"));

        let fs_payload = denials_payload(&rows, Some("fs"), Some(since));
        assert_eq!(fs_payload["total"], 2);
        assert!(
            fs_payload["by_profile"]
                .to_string()
                .contains(SQLITE_FS_BOUNDARY_PROFILE)
        );
        assert!(fs_payload["by_profile"].to_string().contains("restricted"));

        let tool_payload = denials_payload(&rows, Some("tool"), Some(since));
        assert_eq!(tool_payload["total"], 1);

        let sqlite_only = collect_denial_rows(
            &runtime,
            Some(since),
            Some(SQLITE_FS_BOUNDARY_PROFILE),
            Some("codex"),
        )
        .expect("collect filtered sqlite denials");
        assert_eq!(sqlite_only.len(), 1);
        assert_eq!(sqlite_only[0].target(), "/usr/bin/false");
    }

    #[test]
    fn denials_payload_distinguishes_job_runs_from_audit_executions() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let since = Utc::now() - Duration::minutes(5);
        runtime
            .record_audit_event(&orbit_core::AuditEventInsertParams {
                execution_id: "exec-linked-to-run".to_string(),
                command: "tool".to_string(),
                subcommand: Some("orbit.task.update".to_string()),
                tool_name: Some("orbit.task.update".to_string()),
                target_type: Some("task".to_string()),
                target_id: Some("ORB-00001".to_string()),
                role: "codex".to_string(),
                status: AuditEventStatus::Denied,
                exit_code: 1,
                duration_ms: 7,
                working_directory: "/workspace".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: Some("denied by policy".to_string()),
                host: None,
                pid: 123,
                session_id: None,
                task_id: Some("ORB-00001".to_string()),
                job_run_id: Some("jrun-real-policy".to_string()),
                activity_id: Some("agent_implement".to_string()),
                step_index: Some(0),
            })
            .expect("record linked denial");
        runtime
            .record_audit_event(&orbit_core::AuditEventInsertParams {
                execution_id: "audit-task-locks-reserve-denied-test".to_string(),
                command: "task.locks.reserve.denied".to_string(),
                subcommand: None,
                tool_name: Some("orbit.task.locks.reserve".to_string()),
                target_type: Some("task_reservation".to_string()),
                target_id: None,
                role: "admin".to_string(),
                status: AuditEventStatus::Denied,
                exit_code: 1,
                duration_ms: 0,
                working_directory: "/workspace".to_string(),
                arguments_json: Some(
                    json!({
                        "actor": "codex / gpt-5.5",
                        "task_ids": ["ORB-00001"],
                        "files": ["file:crates/orbit-cli/src/lib.rs"],
                        "conflicts": [{
                            "file": "file:crates/orbit-cli/src/lib.rs",
                            "held_by": "task",
                            "held_by_id": "ORB-00002"
                        }]
                    })
                    .to_string(),
                ),
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: None,
                host: None,
                pid: 123,
                session_id: None,
                task_id: None,
                job_run_id: None,
                activity_id: None,
                step_index: None,
            })
            .expect("record task-lock denial");

        let rows = collect_denial_rows(&runtime, Some(since), None, None).expect("collect denials");
        let payload = denials_payload(&rows, None, Some(since));

        let by_run = payload["by_run"].as_array().expect("by_run array");
        assert_eq!(by_run.len(), 1);
        assert_eq!(by_run[0]["run_id"], "jrun-real-policy");
        assert!(
            !payload["by_run"]
                .to_string()
                .contains("audit-task-locks-reserve-denied-test"),
            "audit execution IDs must not be rendered as JobRun IDs"
        );

        let by_execution = payload["by_execution"]
            .as_array()
            .expect("by_execution array");
        assert_eq!(by_execution.len(), 1);
        assert_eq!(
            by_execution[0]["execution_id"],
            "audit-task-locks-reserve-denied-test"
        );

        let recent = payload["recent_denials"]
            .as_array()
            .expect("recent_denials array");
        let invocation = recent
            .iter()
            .find(|row| row["execution_id"] == "audit-task-locks-reserve-denied-test")
            .expect("task-lock row");
        assert_eq!(invocation["identity_type"], "audit_execution");
        assert_eq!(
            invocation["identity_id"],
            "audit-task-locks-reserve-denied-test"
        );
        assert!(invocation["job_run_id"].is_null());
        assert!(invocation["run_id"].is_null());
        assert_eq!(invocation["denial_kind"], "task_lock_reserve");
        assert_eq!(invocation["cause"], "task_lock_conflict");
        assert_eq!(invocation["actor"], "codex / gpt-5.5");
        assert_eq!(invocation["requested_task_ids"][0], "ORB-00001");
        assert_eq!(
            invocation["requested_files"][0],
            "file:crates/orbit-cli/src/lib.rs"
        );
        assert_eq!(invocation["conflicts"][0]["held_by_id"], "ORB-00002");

        let linked = recent
            .iter()
            .find(|row| row["job_run_id"] == "jrun-real-policy")
            .expect("linked JobRun row");
        assert_eq!(linked["identity_type"], "job_run");
        assert_eq!(linked["identity_id"], "jrun-real-policy");

        assert!(
            payload["top_causes"]
                .to_string()
                .contains("task_lock_conflict")
        );
    }
}
