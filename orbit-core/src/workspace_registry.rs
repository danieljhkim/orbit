use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_types::{OrbitError, Workspace, WorkspaceRegistry, WorkspaceStatus};

use crate::fs_utils::atomic_write_text;

/// Returns the global Orbit directory: `~/.orbit/`.
pub fn global_orbit_dir() -> Result<PathBuf, OrbitError> {
    let home = home_dir()?;
    Ok(home.join(".orbit"))
}

/// Returns the path to the global workspace registry file.
pub fn registry_path() -> Result<PathBuf, OrbitError> {
    Ok(global_orbit_dir()?.join("workspaces.json"))
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
    atomic_write_text(path, &content)
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
        .ok_or_else(|| OrbitError::WorkspaceNotFound(id_or_name.to_string()))?;
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
        return Err(OrbitError::WorkspaceNotFound(workspace_id.to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_workspace(id: &str, name: &str, root: &Path) -> Workspace {
        Workspace {
            id: id.to_string(),
            name: name.to_string(),
            root: root.to_path_buf(),
            orbit_dir: root.join(".orbit"),
            git_remote: None,
            base_branch: "main".to_string(),
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn load_missing_registry_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        let registry = load_registry_from(&path).unwrap();
        assert!(registry.workspaces.is_empty());
        assert!(registry.path_overrides.is_empty());
    }

    #[test]
    fn save_and_load_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        let mut registry = WorkspaceRegistry::default();
        let ws = make_workspace("ws_1", "test", dir.path());
        register_workspace(&mut registry, ws).unwrap();
        save_registry_to(&registry, &path).unwrap();

        let loaded = load_registry_from(&path).unwrap();
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].name, "test");
    }

    #[test]
    fn register_duplicate_id_errors() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let ws1 = make_workspace("ws_1", "a", dir.path());
        let ws2 = make_workspace("ws_1", "b", dir.path());
        register_workspace(&mut registry, ws1).unwrap();
        assert!(register_workspace(&mut registry, ws2).is_err());
    }

    #[test]
    fn register_duplicate_name_errors() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let ws1 = make_workspace("ws_1", "same", dir.path());
        let ws2 = make_workspace("ws_2", "same", dir.path());
        register_workspace(&mut registry, ws1).unwrap();
        assert!(register_workspace(&mut registry, ws2).is_err());
    }

    #[test]
    fn remove_workspace_by_name() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let ws = make_workspace("ws_1", "test", dir.path());
        register_workspace(&mut registry, ws).unwrap();
        let removed = remove_workspace(&mut registry, "test").unwrap();
        assert_eq!(removed.id, "ws_1");
        assert!(registry.workspaces.is_empty());
    }

    #[test]
    fn remove_workspace_cleans_path_overrides() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let ws = make_workspace("ws_1", "test", dir.path());
        register_workspace(&mut registry, ws).unwrap();
        set_path_override(&mut registry, PathBuf::from("/some/path"), "ws_1").unwrap();
        assert_eq!(registry.path_overrides.len(), 1);
        remove_workspace(&mut registry, "ws_1").unwrap();
        assert!(registry.path_overrides.is_empty());
    }

    #[test]
    fn remove_nonexistent_workspace_errors() {
        let mut registry = WorkspaceRegistry::default();
        assert!(remove_workspace(&mut registry, "nope").is_err());
    }

    #[test]
    fn find_workspace_by_id_and_name() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let ws = make_workspace("ws_1", "test", dir.path());
        register_workspace(&mut registry, ws).unwrap();
        assert!(find_workspace(&registry, "ws_1").is_some());
        assert!(find_workspace(&registry, "test").is_some());
        assert!(find_workspace(&registry, "nope").is_none());
    }

    #[test]
    fn longest_prefix_match() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();

        let ws1 = make_workspace("ws_1", "project", dir.path());
        let ws2 = make_workspace("ws_2", "subproject", dir.path());
        register_workspace(&mut registry, ws1).unwrap();
        register_workspace(&mut registry, ws2).unwrap();

        registry.path_overrides = HashMap::from([
            (PathBuf::from("/home/user/projects"), "ws_1".to_string()),
            (PathBuf::from("/home/user/projects/sub"), "ws_2".to_string()),
        ]);

        // Exact match on deeper path → ws_2
        let found = find_workspace_by_path(&registry, Path::new("/home/user/projects/sub/file"));
        assert_eq!(found.unwrap().id, "ws_2");

        // Match on parent path → ws_1
        let found = find_workspace_by_path(&registry, Path::new("/home/user/projects/other"));
        assert_eq!(found.unwrap().id, "ws_1");

        // No match
        let found = find_workspace_by_path(&registry, Path::new("/elsewhere"));
        assert!(found.is_none());
    }

    #[test]
    fn set_path_override_validates_workspace_exists() {
        let mut registry = WorkspaceRegistry::default();
        let result = set_path_override(&mut registry, PathBuf::from("/tmp"), "ws_nope");
        assert!(result.is_err());
    }

    #[test]
    fn validate_marks_missing_roots_invalid() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("exists");
        std::fs::create_dir_all(&existing).unwrap();

        let mut registry = WorkspaceRegistry::default();
        let ws1 = make_workspace("ws_1", "exists", &existing);
        let ws2 = make_workspace("ws_2", "gone", Path::new("/nonexistent/path"));
        register_workspace(&mut registry, ws1).unwrap();
        register_workspace(&mut registry, ws2).unwrap();

        validate_workspaces(&mut registry);
        assert_eq!(registry.workspaces[0].status, WorkspaceStatus::Active);
        assert_eq!(registry.workspaces[1].status, WorkspaceStatus::Invalid);
    }

    #[test]
    fn validate_recovers_previously_invalid() {
        let dir = tempdir().unwrap();
        let mut registry = WorkspaceRegistry::default();
        let mut ws = make_workspace("ws_1", "test", dir.path());
        ws.status = WorkspaceStatus::Invalid;
        register_workspace(&mut registry, ws).unwrap();

        validate_workspaces(&mut registry);
        assert_eq!(registry.workspaces[0].status, WorkspaceStatus::Active);
    }

    #[test]
    fn atomic_write_no_leftover_tmp() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspaces.json");
        save_registry_to(&WorkspaceRegistry::default(), &path).unwrap();
        assert!(path.exists());
        assert!(!dir.path().join("workspaces.json.tmp").exists());
    }
}
