//! Unit tests for multi-workspace state and default-workspace resolution
//! (ORB-00030).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};
use orbit_core::OrbitRuntime;

use crate::default_workspace_for_cwd;
use crate::state::{DashboardState, WsEntry};

fn workspace(id: &str, root: &str, status: WorkspaceStatus) -> Workspace {
    let now = Utc::now();
    Workspace {
        id: id.to_string(),
        name: id.to_string(),
        root: PathBuf::from(root),
        orbit_dir: PathBuf::from(root).join(".orbit"),
        git_remote: None,
        base_branch: "main".to_string(),
        status,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn single_mode_exposes_one_default_workspace() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let state = DashboardState::single(Arc::new(runtime));

    assert_eq!(state.entries().len(), 1);
    assert_eq!(state.default_workspace(), Some("default"));
    assert!(state.runtime_for("default").is_ok());
    assert!(state.runtime_for("unknown").is_err());
}

#[test]
fn global_mode_rejects_inactive_and_unknown_workspaces() {
    let entries = vec![WsEntry {
        id: "stale".to_string(),
        name: "stale".to_string(),
        repo_root: PathBuf::from("/nonexistent"),
        orbit_dir: PathBuf::from("/nonexistent/.orbit"),
        active: false,
    }];
    let state = DashboardState::global(PathBuf::from("/nonexistent"), entries, None);

    // Inactive entries are listed but never built.
    assert_eq!(state.entries().len(), 1);
    assert!(state.runtime_for("stale").is_err());
    // Unknown ids are rejected outright.
    assert!(state.runtime_for("ghost").is_err());
    assert_eq!(state.default_workspace(), None);
}

#[test]
fn default_workspace_for_cwd_picks_longest_active_prefix() {
    let registry = WorkspaceRegistry {
        workspaces: vec![
            workspace("outer", "/repos", WorkspaceStatus::Active),
            workspace("inner", "/repos/inner", WorkspaceStatus::Active),
            workspace("stale", "/repos/inner/sub", WorkspaceStatus::Invalid),
        ],
        path_overrides: Default::default(),
    };

    // Deepest active workspace wins; the still-deeper inactive one is ignored.
    assert_eq!(
        default_workspace_for_cwd(&registry, Path::new("/repos/inner/sub/pkg")),
        Some("inner".to_string())
    );
    // Outside any registered root -> no default (frontend opens the aggregate).
    assert_eq!(
        default_workspace_for_cwd(&registry, Path::new("/elsewhere")),
        None
    );
}
