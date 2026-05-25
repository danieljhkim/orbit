//! Scoreboard endpoint: per-agent stats joined with metrics extras and denials.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use chrono::{Duration, Utc};
use orbit_core::OrbitRuntime;
use orbit_core::scoreboard_summary::ScoreboardWindow;
use serde::Deserialize;
use serde_json::json;

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

pub(super) async fn scoreboard(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(query): Query<ScoreboardQuery>,
) -> Response {
    let window = match query.window.as_deref() {
        None => ScoreboardWindow::All,
        Some(raw) => match ScoreboardWindow::from_str(raw) {
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
    // Denials honor the same window as the rest of the scoreboard so the
    // dashboard column lines up with the windowed counts.
    let metrics_extras = compute_metrics_extras(&runtime).unwrap_or_default();
    let since_window = window.duration().map(|d| Utc::now() - d);
    let denials_by_role = runtime
        .audit_denials_by_role(since_window.as_ref())
        .unwrap_or_default();
    let denial_map: BTreeMap<String, i64> = denials_by_role.into_iter().collect();

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
            }
        }
        // Surface metrics-only agents so retries/durations show even when no
        // task or token row exists for them yet.
        for (key, extras) in &metrics_extras {
            if existing_keys.iter().any(|k| k == key) {
                continue;
            }
            let denials = lookup_denials_for_agent(&denial_map, key);
            agents.insert(
                key.clone(),
                json!({
                    "tasks_completed": 0,
                    "tokens": { "total": 0, "output": 0 },
                    "duels": { "wins": 0, "losses": 0, "participated": 0 },
                    "pr": { "review_comments": 0, "merged_clean": 0, "merged_with_revision": 0 },
                    "task_review": { "threads": 0 },
                    "friction": { "reported": 0 },
                    "tool_calls": 0,
                    "failed_tool_calls": 0,
                    "avg_step_duration_ms": extras.avg_duration_ms,
                    "retries": extras.retry_count,
                    "p95_wall_clock_ms": extras.p95_duration_ms,
                    "denials": denials,
                }),
            );
        }
    }

    Json(value).into_response()
}

/// Per-agent extras derived from `MetricsEntry` JSONL.
#[derive(Debug, Clone, Default)]
struct MetricsExtras {
    avg_duration_ms: i64,
    p95_duration_ms: i64,
    retry_count: i64,
}

fn compute_metrics_extras(
    runtime: &OrbitRuntime,
) -> Result<BTreeMap<String, MetricsExtras>, orbit_core::OrbitError> {
    use orbit_common::types::ActorIdentity;

    let now = Utc::now();
    let mut months = Vec::new();
    months.push(now.format("%Y-%m").to_string());
    if let Some(prev) = now.checked_sub_signed(Duration::days(31)) {
        let key = prev.format("%Y-%m").to_string();
        if !months.contains(&key) {
            months.push(key);
        }
    }

    let mut by_actor: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut retries: BTreeMap<String, i64> = BTreeMap::new();
    for month in &months {
        let entries = match runtime.read_metrics_entries(month) {
            Ok(e) => e,
            Err(orbit_core::OrbitError::InvalidInput(_)) => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
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
