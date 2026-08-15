use std::path::{Path, PathBuf};

use orbit_common::types::{OrbitError, WorkspaceRegistry};
use orbit_common::utility::fs::atomic_write_text;
pub use orbit_common::utility::path::global_orbit_dir;

use super::{WorkspaceRegistryHostContext, parse_workspace_registry, validate_workspace_registry};
use crate::{HostIdentityState, inspect_host_identity};

/// Return the path to the machine-global workspace registry.
pub fn registry_path() -> Result<PathBuf, OrbitError> {
    Ok(registry_path_for(&global_orbit_dir()?))
}

/// Return the workspace registry path under an already-resolved global root.
pub fn registry_path_for(global_root: &Path) -> PathBuf {
    global_root.join("workspaces.json")
}

/// Load the machine-global workspace registry.
pub fn load_registry() -> Result<WorkspaceRegistry, OrbitError> {
    load_registry_from(&registry_path()?)
}

/// Load, migrate, and validate a registry from an explicit path.
pub fn load_registry_from(path: &Path) -> Result<WorkspaceRegistry, OrbitError> {
    load_registry_from_with_writer(path, write_registry)
}

pub(crate) fn load_registry_from_with_writer(
    path: &Path,
    writer: impl FnOnce(&WorkspaceRegistry, &Path) -> Result<(), OrbitError>,
) -> Result<WorkspaceRegistry, OrbitError> {
    if !path.exists() {
        return Ok(WorkspaceRegistry::default());
    }
    let content =
        std::fs::read_to_string(path).map_err(|error| OrbitError::Io(error.to_string()))?;
    let context = registry_host_context(path)?;
    let (registry, migrated) = parse_workspace_registry(&content, &context)?;
    if migrated {
        writer(&registry, path)?;
    }
    Ok(registry)
}

/// Save the machine-global workspace registry atomically.
pub fn save_registry(registry: &WorkspaceRegistry) -> Result<(), OrbitError> {
    save_registry_to(registry, &registry_path()?)
}

/// Validate and atomically save a registry to an explicit path.
pub fn save_registry_to(registry: &WorkspaceRegistry, path: &Path) -> Result<(), OrbitError> {
    let context = registry_host_context(path)?;
    let mut canonical = registry.clone();
    validate_workspace_registry(&mut canonical, &context)?;
    write_registry(&canonical, path)
}

fn registry_host_context(path: &Path) -> Result<WorkspaceRegistryHostContext, OrbitError> {
    let global_root = path.parent().ok_or_else(|| {
        OrbitError::WorkspaceError(format!(
            "registry path '{}' has no parent directory",
            path.display()
        ))
    })?;
    match inspect_host_identity(global_root)? {
        HostIdentityState::Present(identity) => Ok(WorkspaceRegistryHostContext {
            machine_id: Some(identity.machine_id),
            host_id: Some(identity.host_id),
        }),
        HostIdentityState::Legacy { .. } | HostIdentityState::Absent => {
            Ok(WorkspaceRegistryHostContext::default())
        }
    }
}

fn write_registry(registry: &WorkspaceRegistry, path: &Path) -> Result<(), OrbitError> {
    let content = serde_json::to_string_pretty(registry)
        .map_err(|error| OrbitError::WorkspaceError(format!("serialize registry: {error}")))?;
    atomic_write_text(path, &content).map_err(Into::into)
}
