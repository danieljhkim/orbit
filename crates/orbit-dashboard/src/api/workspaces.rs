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
    // Reflect native registry mutations (add/remove/rebind) before listing.
    state.refresh();
    let default = state.default_workspace();
    let home = home_dir();
    let values: Vec<Value> = state
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "name": entry.name,
                "root": abbreviate_home(&entry.repo_root, home.as_deref()),
                "orbit_dir": abbreviate_home(&entry.orbit_dir, home.as_deref()),
                "status": if entry.active { "active" } else { "invalid" },
                "is_default": default.as_deref() == Some(entry.id.as_str()),
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
    // Reflect native registry mutations before aggregating across workspaces.
    state.refresh();
    let home = home_dir();
    let mut all = Vec::new();
    let entries = state.entries();
    for entry in entries.iter().filter(|entry| entry.active) {
        let Ok(runtime) = state.runtime_for(&entry.id) else {
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
