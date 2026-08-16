//! Global (cross-workspace) endpoints (ORB-00030).
//!
//! These handlers take the whole [`DashboardState`] rather than a single
//! workspace runtime (via the [`Ws`](crate::state::Ws) extractor), because they
//! describe or aggregate across every servable workspace. In single mode they
//! degrade gracefully: `/api/workspaces` reports the one synthetic entry and
//! `/api/tasks/all` returns that workspace's tasks.

use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::response::{IntoResponse, Json, Response};
use orbit_core::DEFAULT_TASK_LIST_LIMIT;
use serde_json::{Value, json};

use super::server_error;
use super::tasks::list_tasks_json;
use crate::state::DashboardState;

/// `GET /api/workspaces` — list every workspace the dashboard can serve, with
/// the currently-selected default flagged.
///
/// Filesystem paths (`root`, `orbit_dir`) are home-abbreviated to `~` so the
/// frontend can render the selected workspace's location directly, without
/// needing to know the server's home directory (ORB-00037).
pub(super) async fn list_workspaces(State(state): State<DashboardState>) -> Response {
    // Refresh and pin one snapshot so the listing and its `is_default` flags all
    // reflect the same generation (add/remove/rebind observed atomically).
    let pinned = state.pin();
    let default = pinned.default_workspace();
    let home = home_dir();
    let values: Vec<Value> = pinned
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "name": entry.name,
                "root": abbreviate_home(&entry.repo_root, home.as_deref()),
                "orbit_dir": abbreviate_home(&entry.orbit_dir, home.as_deref()),
                "status": if entry.active { "active" } else { "invalid" },
                "is_default": default == Some(entry.id.as_str()),
            })
        })
        .collect();
    Json(Value::Array(values)).into_response()
}

/// `GET /api/tasks/all` — dashboard tasks aggregated across active workspaces.
///
/// Each task object is tagged with its owning workspace (`workspace_id` /
/// `workspace_name`, plus the home-abbreviated `workspace_root` path added in
/// ORB-00037) so the frontend can badge the row and show the full location in
/// the task's Details box. Inactive (stale-path) workspaces are skipped, as are
/// any that fail to open — the aggregate view stays available even when one
/// workspace is broken.
pub(super) async fn list_all_tasks(State(state): State<DashboardState>) -> Response {
    // Refresh and pin one snapshot so every task's workspace tag (id, name,
    // root) and the runtime it was listed from come from the same generation —
    // never old metadata spliced onto a runtime resolved from a newer binding.
    let pinned = state.pin();
    let home = home_dir();
    let mut all = Vec::new();
    for entry in pinned.entries().iter().filter(|entry| entry.active) {
        let Ok(runtime) = pinned.runtime_for(&entry.id) else {
            continue;
        };
        let values = match list_tasks_json(&runtime) {
            Ok(values) => values,
            Err(e) => return server_error(e),
        };
        let workspace_root = abbreviate_home(&entry.repo_root, home.as_deref());
        for mut value in values {
            if let Value::Object(map) = &mut value {
                map.insert("workspace_id".to_string(), json!(entry.id));
                map.insert("workspace_name".to_string(), json!(entry.name));
                map.insert("workspace_root".to_string(), json!(workspace_root));
            }
            all.push(value);
        }
    }
    // Each workspace already contributes its newest tasks; re-sort the union so
    // the aggregate is globally newest-first and bounded to the same default
    // limit as every other task-listing surface (ORB-10310). `created_at` is a
    // fixed-format UTC RFC 3339 string, so lexical order is chronological; task
    // ID breaks timestamp ties ascending.
    all.sort_by(|a, b| {
        let created = |value: &Value| {
            value
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let id = |value: &Value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        created(b).cmp(&created(a)).then_with(|| id(a).cmp(&id(b)))
    });
    all.truncate(DEFAULT_TASK_LIST_LIMIT);
    Json(Value::Array(all)).into_response()
}

/// Render a filesystem path for display, collapsing the user's home directory
/// to `~` (e.g. `/home/dan/ws/orbit` → `~/ws/orbit`). Paths outside `home`, an
/// unset `home`, or an empty path (the single-mode synthetic entry) are rendered
/// verbatim. ORB-00037.
pub(super) fn abbreviate_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home {
        if path == home {
            return "~".to_string();
        }
        if let Ok(rest) = path.strip_prefix(home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// The current user's home directory from `$HOME`, ignoring an empty value.
/// Used only to abbreviate paths for display; absence just disables the `~`
/// collapse (paths render verbatim).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}
