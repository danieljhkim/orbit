//! Scoreboard endpoint: per-agent stats joined with metrics extras and denials.

use std::collections::BTreeMap;

use crate::state::Ws;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Datelike, Utc};
use orbit_cmd::DiagnosticsCommands;
use orbit_core::scoreboard_summary::ScoreboardWindow;
use orbit_core::{FailureIncidentQuery, OrbitRuntime};
use serde::Deserialize;
use serde_json::json;

use super::incidents::{ActorFailureRollup, ROLLUP_SCAN_LIMIT, agent_family_key, rollup_by_actor};
use super::server_error;

/// Query-string shape for `GET /api/scoreboard`.
///
/// `?window=<1h|24h|7d|30d|all>` scopes the summary. Missing param keeps
/// the legacy lifetime behavior. Unknown values produce HTTP 400.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ScoreboardQuery {
    #[serde(default)]
    pub(super) window: Option<String>,
}

pub(super) async fn scoreboard(Ws(runtime): Ws, Query(query): Query<ScoreboardQuery>) -> Response {
    let window = match query.window.as_deref() {
        None => ScoreboardWindow::All,
        Some(raw) => match raw.parse::<ScoreboardWindow>() {
            Ok(w) => w,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        },
    };

    let summary = match runtime.generate_scoreboard_summary(Some(window)) {
        Ok(s) => s,
        Err(e) => return server_error(e),
    };
    let mut value = match serde_json::to_value(&summary) {
        Ok(v) => v,
        Err(e) => return server_error(orbit_core::OrbitError::Store(e.to_string())),
    };

    // Join MetricsEntry-derived per-actor stats and audit denials. Errors are
    // logged-and-swallowed so the existing scoreboard surface still renders if
    // a side log is missing or malformed.
    //
    // One `now`/`since_window` pair scopes every join below (metrics extras,
    // denials, failure rollup) so the response mixes only compatible
    // populations for the requested window.
    let now = Utc::now();
    let since_window = window.duration().map(|d| now - d);
    let metrics_extras = compute_metrics_extras(&runtime, since_window, now).unwrap_or_default();
    let denials_by_role = runtime
        .audit_denials_by_role(since_window.as_ref())
        .unwrap_or_default();
    let denial_map: BTreeMap<String, i64> = denials_by_role.into_iter().collect();

    // ORB-10871: a repeated failure burst is one incident, not one failure per
    // raw row. The scoreboard keeps `failed_tool_calls` (the raw count) and
    // gains the grouped counts beside it, over the same window, so an operator
    // reads "how many things went wrong" and "how much evidence there is" as
    // two distinct numbers.
    let failure_rollup = runtime
        .audit_failure_incidents(&FailureIncidentQuery {
            since: since_window,
            max_events: ROLLUP_SCAN_LIMIT,
            ..Default::default()
        })
        .map(|report| rollup_by_actor(&report))
        .unwrap_or_default();

    if let Some(agents) = value.get_mut("agents").and_then(|v| v.as_object_mut()) {
        // Collect all agent keys upfront so we can also surface metrics rows
        // that have no scoreboard counterpart yet.
        let existing_keys: Vec<String> = agents.keys().cloned().collect();
        for key in &existing_keys {
            let extras = metrics_extras
                .get(key.as_str())
                .cloned()
                .unwrap_or_default();
            let denials = lookup_denials_for_agent(&denial_map, key);
            let failures = lookup_failures_for_agent(&failure_rollup, key);
            if let Some(obj) = agents.get_mut(key.as_str()).and_then(|v| v.as_object_mut()) {
                obj.insert(
                    "avg_step_duration_ms".to_string(),
                    json!(extras.avg_duration_ms),
                );
                obj.insert("retries".to_string(), json!(extras.retry_count));
                obj.insert(
                    "p95_wall_clock_ms".to_string(),
                    json!(extras.p95_duration_ms),
                );
                obj.insert("denials".to_string(), json!(denials));
                obj.insert("failure_incidents".to_string(), json!(failures.incidents));
                obj.insert(
                    "unexpected_failure_incidents".to_string(),
                    json!(failures.unexpected_incidents),
                );
                obj.insert(
                    "failure_incident_events".to_string(),
                    json!(failures.events),
                );
            }
        }
        // Surface metrics-only agents so retries/durations show even when no
        // task or token row exists for them yet.
        for (key, extras) in &metrics_extras {
            if existing_keys.iter().any(|k| k == key) {
                continue;
            }
            let denials = lookup_denials_for_agent(&denial_map, key);
            let failures = lookup_failures_for_agent(&failure_rollup, key);
            agents.insert(
                key.clone(),
                json!({
                    "tasks_completed": 0,
                    "tokens": { "total": 0, "output": 0 },
                    "pr": { "review_comments": 0, "merged_clean": 0, "merged_with_revision": 0 },
                    "task_review": { "threads": 0 },
                    "friction": { "reported": 0 },
                    "tool_calls": 0,
                    "failed_tool_calls": 0,
                    "avg_step_duration_ms": extras.avg_duration_ms,
                    "retries": extras.retry_count,
                    "p95_wall_clock_ms": extras.p95_duration_ms,
                    "denials": denials,
                    "failure_incidents": failures.incidents,
                    "unexpected_failure_incidents": failures.unexpected_incidents,
                    "failure_incident_events": failures.events,
                }),
            );
        }
    }

    Json(value).into_response()
}

/// Per-agent extras derived from `MetricsEntry` JSONL.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct MetricsExtras {
    pub(super) avg_duration_ms: i64,
    pub(super) p95_duration_ms: i64,
    pub(super) retry_count: i64,
}

// `pub(super)`: exercised directly by the sibling `api/tests/scoreboard.rs`
// unit tests (window/month-boundary arithmetic), per the crate's sibling
// test-layout convention — see docs/design-patterns/test_layout.md.
pub(super) fn compute_metrics_extras(
    runtime: &OrbitRuntime,
    since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<BTreeMap<String, MetricsExtras>, orbit_core::OrbitError> {
    use orbit_types::identity::ActorIdentity;

    // A finite window only needs its own partitions; lifetime (`since ==
    // None`) enumerates every partition that has ever been written so
    // records older than a couple of months are still counted.
    let months = match since {
        Some(since) => months_in_range(since, now),
        None => runtime.list_metrics_months()?,
    };

    let mut by_actor: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut retries: BTreeMap<String, i64> = BTreeMap::new();
    for month in &months {
        let entries = match runtime.read_metrics_entries(month) {
            Ok(e) => e,
            Err(orbit_core::OrbitError::InvalidInput(_)) => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            // A month partition covers a superset of the requested range
            // (e.g. the first/last day of the month), so re-check each
            // record against the exact boundary.
            if since.is_some_and(|since| entry.ts < since) || entry.ts > now {
                continue;
            }
            let key = match &entry.actor_identity {
                ActorIdentity::Agent { model } if !model.is_empty() => model.clone(),
                ActorIdentity::Human { label } if !label.is_empty() => label.clone(),
                _ => continue,
            };
            *retries.entry(key.clone()).or_insert(0) += entry.retry_count as i64;
            if let Some(d) = entry.step_duration_ms {
                by_actor.entry(key).or_default().push(d);
            }
        }
    }

    let mut out: BTreeMap<String, MetricsExtras> = BTreeMap::new();
    for (key, durations) in by_actor {
        let mut sorted = durations.clone();
        sorted.sort_unstable();
        let sum: u128 = sorted.iter().map(|d| *d as u128).sum();
        let avg = if sorted.is_empty() {
            0
        } else {
            (sum / sorted.len() as u128) as i64
        };
        let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let idx = idx.min(sorted.len()).saturating_sub(1);
        let p95 = sorted.get(idx).copied().unwrap_or(0) as i64;
        let retry = retries.remove(&key).unwrap_or(0);
        out.insert(
            key,
            MetricsExtras {
                avg_duration_ms: avg,
                p95_duration_ms: p95,
                retry_count: retry,
            },
        );
    }
    // Carry over retries-only actors that had no duration samples.
    for (key, count) in retries {
        out.entry(key).or_insert(MetricsExtras {
            avg_duration_ms: 0,
            p95_duration_ms: 0,
            retry_count: count,
        });
    }
    Ok(out)
}

/// Ascending `YYYY-MM` partitions spanning `since..=now`, inclusive of both
/// endpoints' months. Steps by calendar month rather than a fixed day offset
/// so a window whose start falls on an early-month date (e.g. subtracting 30
/// days from March 1st) still includes every month it actually touches.
pub(super) fn months_in_range(since: DateTime<Utc>, now: DateTime<Utc>) -> Vec<String> {
    let (mut year, mut month) = (since.year(), since.month());
    let (end_year, end_month) = (now.year(), now.month());

    let mut months = Vec::new();
    loop {
        months.push(format!("{year:04}-{month:02}"));
        if year > end_year || (year == end_year && month >= end_month) {
            break;
        }
        if month == 12 {
            month = 1;
            year += 1;
        } else {
            month += 1;
        }
    }
    months
}

/// Looks up an agent's grouped failure counts. The rollup is already keyed by
/// canonical agent family, so a scoreboard key that is itself a model label
/// (metrics-only rows can be) is normalized the same way before matching.
fn lookup_failures_for_agent(
    rollup: &BTreeMap<String, ActorFailureRollup>,
    agent_key: &str,
) -> ActorFailureRollup {
    if let Some(found) = rollup.get(agent_key) {
        return *found;
    }
    rollup
        .get(&agent_family_key(agent_key))
        .copied()
        .unwrap_or_default()
}

/// Looks up the SQLite per-role denials for a scoreboard agent key. The audit
/// schema stores `role` as a free-form string (often the bare agent name), so
/// we accept either a direct match or a model-prefix match.
fn lookup_denials_for_agent(map: &BTreeMap<String, i64>, agent_key: &str) -> i64 {
    if let Some(v) = map.get(agent_key) {
        return *v;
    }
    // Match by leading agent-name portion of `agent / model` keys.
    if let Some(idx) = agent_key.find(" / ") {
        let head = &agent_key[..idx];
        if let Some(v) = map.get(head) {
            return *v;
        }
    }
    0
}
