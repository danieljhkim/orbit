//! Global (cross-workspace) endpoints (ORB-00030).
//!
//! These handlers take the whole [`DashboardState`] rather than a single
//! workspace runtime (via the [`Ws`](crate::state::Ws) extractor), because they
//! describe or aggregate across every servable workspace. In single mode they
//! degrade gracefully: `/api/workspaces` reports the one synthetic entry and
//! `/api/tasks/all` returns that workspace's tasks.

use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{Value, json};

use super::server_error;
use super::tasks::list_tasks_json;
use crate::state::DashboardState;

/// `GET /api/workspaces` — list every workspace the dashboard can serve, with
/// the currently-selected default flagged.
pub(super) async fn list_workspaces(State(state): State<DashboardState>) -> Response {
    let default = state.default_workspace();
    let values: Vec<Value> = state
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "name": entry.name,
                "root": entry.repo_root,
                "status": if entry.active { "active" } else { "invalid" },
                "is_default": Some(entry.id.as_str()) == default,
            })
        })
        .collect();
    Json(Value::Array(values)).into_response()
}

/// `GET /api/tasks/all` — dashboard tasks aggregated across active workspaces.
///
/// Each task object is tagged with its owning workspace (`workspace_id` /
/// `workspace_name`) so the frontend can render a workspace column. Inactive
/// (stale-path) workspaces are skipped, as are any that fail to open — the
/// aggregate view stays available even when one workspace is broken.
pub(super) async fn list_all_tasks(State(state): State<DashboardState>) -> Response {
    let mut all = Vec::new();
    for entry in state.entries().iter().filter(|entry| entry.active) {
        let Ok(runtime) = state.runtime_for(&entry.id) else {
            continue;
        };
        let values = match list_tasks_json(&runtime) {
            Ok(values) => values,
            Err(e) => return server_error(e),
        };
        for mut value in values {
            if let Value::Object(map) = &mut value {
                map.insert("workspace_id".to_string(), json!(entry.id));
                map.insert("workspace_name".to_string(), json!(entry.name));
            }
            all.push(value);
        }
    }
    Json(Value::Array(all)).into_response()
}
