use std::path::Path;

use orbit_common::OrbitError;
use orbit_common::fs::io::create_dir_symlink;

use crate::driver::sqlite::task_registry::{ProjectionRebuildResult, TaskRegistryStore};
use crate::fs::path_safety::normalize_path;

pub(crate) fn rebuild_projection(
    registry: &TaskRegistryStore,
    workspace_orbit_dir: &Path,
    workspace_id: &str,
) -> Result<ProjectionRebuildResult, OrbitError> {
    let checkout = registry.require_workspace_checkout(workspace_id)?;
    if normalize_path(workspace_orbit_dir) != normalize_path(&checkout.orbit_dir) {
        return Err(OrbitError::InvalidInput(format!(
            "workspace '{}' checkout is bound to '{}', not '{}'",
            workspace_id,
            checkout.orbit_dir.display(),
            workspace_orbit_dir.display()
        )));
    }
    let projection_dir = workspace_orbit_dir.join("tasks");
    std::fs::create_dir_all(&projection_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    let tasks = registry.tasks_for_workspace(workspace_id)?;
    let mut result = ProjectionRebuildResult {
        projected: 0,
        repaired: 0,
        degraded_reason: None,
    };
    for task in tasks {
        let link_path = projection_dir.join(&task.task_id);
        let target = task.canonical_path;
        match std::fs::symlink_metadata(&link_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let current =
                    std::fs::read_link(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                if normalize_path(&current) != normalize_path(&target) {
                    std::fs::remove_file(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                    create_projection_symlink(&target, &link_path, &mut result)?;
                    result.repaired += 1;
                } else {
                    result.projected += 1;
                }
            }
            Ok(_) => {
                return Err(OrbitError::Store(format!(
                    "projection entry '{}' already exists and is not a symlink",
                    link_path.display()
                )));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                create_projection_symlink(&target, &link_path, &mut result)?;
            }
            Err(err) => return Err(OrbitError::Io(err.to_string())),
        }
        if result.degraded_reason.is_some() {
            return Ok(result);
        }
    }
    Ok(result)
}

pub(super) fn create_projection_symlink(
    target: &Path,
    link_path: &Path,
    result: &mut ProjectionRebuildResult,
) -> Result<(), OrbitError> {
    match create_dir_symlink(target, link_path) {
        Ok(()) => {
            result.projected += 1;
            Ok(())
        }
        Err(err) if is_symlink_degraded_error(&err) => {
            result.degraded_reason = Some(format!(
                "directory symlinks are unavailable for '{}': {err}",
                link_path.display()
            ));
            Ok(())
        }
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn is_symlink_degraded_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
    )
}

pub(crate) fn ensure_projection_entry_removable(
    workspace_orbit_dir: &Path,
    task_id: &str,
) -> Result<(), OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match std::fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

pub(crate) fn remove_projection_entry(
    workspace_orbit_dir: &Path,
    task_id: &str,
) -> Result<bool, OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match std::fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::remove_file(&projection_path)
                .map_err(|err| OrbitError::Io(err.to_string()))?;
            Ok(true)
        }
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}
