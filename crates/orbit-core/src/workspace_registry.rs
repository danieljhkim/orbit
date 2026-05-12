use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    NotFoundKind, OrbitError, Workspace, WorkspaceRegistry, WorkspaceStatus,
};

use orbit_common::utility::fs::atomic_write_text;

/// Returns the global Orbit directory: `~/.orbit/`.
pub fn global_orbit_dir() -> Result<PathBuf, OrbitError> {
    let home = home_dir()?;
    Ok(home.join(".orbit"))
}

/// Returns the path to the global workspace registry file.
pub fn registry_path() -> Result<PathBuf, OrbitError> {
    Ok(registry_path_for(&global_orbit_dir()?))
}

/// Returns the workspace registry path for an already-resolved Orbit root.
pub fn registry_path_for(global_root: &Path) -> PathBuf {
    global_root.join("workspaces.json")
}

/// Loads the workspace registry from `~/.orbit/workspaces.json`.
/// Returns an empty registry if the file does not exist.
pub fn load_registry() -> Result<WorkspaceRegistry, OrbitError> {
    load_registry_from(&registry_path()?)
}

/// Loads the workspace registry from a specific path (for testing).
pub fn load_registry_from(path: &Path) -> Result<WorkspaceRegistry, OrbitError> {
    if !path.exists() {
        return Ok(WorkspaceRegistry::default());
    }
    let content = std::fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    serde_json::from_str(&content)
        .map_err(|e| OrbitError::WorkspaceError(format!("invalid registry: {e}")))
}

/// Saves the workspace registry atomically.
pub fn save_registry(registry: &WorkspaceRegistry) -> Result<(), OrbitError> {
    save_registry_to(registry, &registry_path()?)
}

/// Saves the workspace registry to a specific path (for testing).
pub fn save_registry_to(registry: &WorkspaceRegistry, path: &Path) -> Result<(), OrbitError> {
    let content = serde_json::to_string_pretty(registry)
        .map_err(|e| OrbitError::WorkspaceError(format!("failed to serialize registry: {e}")))?;
    atomic_write_text(path, &content).map_err(Into::into)
}

/// Registers a new workspace. Errors if a workspace with the same id or name already exists.
pub fn register_workspace(
    registry: &mut WorkspaceRegistry,
    ws: Workspace,
) -> Result<(), OrbitError> {
    if registry.workspaces.iter().any(|w| w.id == ws.id) {
        return Err(OrbitError::WorkspaceError(format!(
            "workspace with id '{}' already exists",
            ws.id
        )));
    }
    if registry.workspaces.iter().any(|w| w.name == ws.name) {
        return Err(OrbitError::WorkspaceError(format!(
            "workspace with name '{}' already exists",
            ws.name
        )));
    }
    registry.workspaces.push(ws);
    Ok(())
}

/// Removes a workspace by id or name. Returns the removed workspace.
pub fn remove_workspace(
    registry: &mut WorkspaceRegistry,
    id_or_name: &str,
) -> Result<Workspace, OrbitError> {
    let idx = registry
        .workspaces
        .iter()
        .position(|w| w.id == id_or_name || w.name == id_or_name)
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::Workspace, id_or_name.to_string()))?;
    let removed = registry.workspaces.remove(idx);
    // Also remove any path overrides pointing to this workspace
    registry
        .path_overrides
        .retain(|_, ws_id| ws_id != &removed.id);
    Ok(removed)
}

/// Finds a workspace by id or name.
pub fn find_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    id_or_name: &str,
) -> Option<&'a Workspace> {
    registry
        .workspaces
        .iter()
        .find(|w| w.id == id_or_name || w.name == id_or_name)
}

/// Finds the workspace for a given path using longest prefix match on `path_overrides`.
pub fn find_workspace_by_path<'a>(
    registry: &'a WorkspaceRegistry,
    cwd: &Path,
) -> Option<&'a Workspace> {
    let mut best_match: Option<(&PathBuf, &String)> = None;

    for (override_path, ws_id) in &registry.path_overrides {
        if cwd.starts_with(override_path) {
            match best_match {
                Some((current_best, _))
                    if override_path.as_os_str().len() > current_best.as_os_str().len() =>
                {
                    best_match = Some((override_path, ws_id));
                }
                None => {
                    best_match = Some((override_path, ws_id));
                }
                _ => {}
            }
        }
    }

    let (_, ws_id) = best_match?;
    registry.workspaces.iter().find(|w| &w.id == ws_id)
}

/// Sets a path override binding a directory to a workspace.
pub fn set_path_override(
    registry: &mut WorkspaceRegistry,
    path: PathBuf,
    workspace_id: &str,
) -> Result<(), OrbitError> {
    // Verify the workspace exists
    if !registry.workspaces.iter().any(|w| w.id == workspace_id) {
        return Err(OrbitError::not_found(
            NotFoundKind::Workspace,
            workspace_id.to_string(),
        ));
    }
    registry
        .path_overrides
        .insert(path, workspace_id.to_string());
    Ok(())
}

/// Validates all workspaces in the registry, marking those whose root no longer exists as invalid.
pub fn validate_workspaces(registry: &mut WorkspaceRegistry) {
    let now = Utc::now();
    for ws in &mut registry.workspaces {
        if ws.root.exists() {
            if ws.status == WorkspaceStatus::Invalid {
                ws.status = WorkspaceStatus::Active;
                ws.updated_at = now;
            }
        } else if ws.status == WorkspaceStatus::Active {
            ws.status = WorkspaceStatus::Invalid;
            ws.updated_at = now;
        }
    }
}

fn home_dir() -> Result<PathBuf, OrbitError> {
    dirs_or_fallback()
}

fn dirs_or_fallback() -> Result<PathBuf, OrbitError> {
    // Try HOME env first (works in tests), then platform default
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    #[cfg(windows)]
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !profile.is_empty() {
            return Ok(PathBuf::from(profile));
        }
    }
    Err(OrbitError::WorkspaceError(
        "cannot determine home directory".to_string(),
    ))
}
