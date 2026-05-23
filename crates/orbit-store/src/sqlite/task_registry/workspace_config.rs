use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::atomic_write_text;
use serde::{Deserialize, Serialize};

use super::CONFIG_SCHEMA_VERSION;
use super::types::WorkspaceConfig;
use super::workspace_id::{sanitize_slug, validate_workspace_id, workspace_id_candidate};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceConfigDoc {
    schema_version: u32,
    workspace_id: String,
}

pub fn task_registry_path(global_root: &Path) -> PathBuf {
    global_root.join("tasks").join("index.sqlite")
}

pub fn home_task_workspace_dir(global_root: &Path, workspace_id: &str) -> PathBuf {
    global_root
        .join("tasks")
        .join("workspaces")
        .join(workspace_id)
}

pub fn workspace_config_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("config.yaml")
}

pub fn read_workspace_config(orbit_dir: &Path) -> Result<WorkspaceConfig, OrbitError> {
    read_workspace_config_optional(orbit_dir)?.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "workspace config is missing: {}",
            workspace_config_path(orbit_dir).display()
        ))
    })
}

pub fn workspace_id_for_orbit_dir(orbit_dir: &Path) -> Result<String, OrbitError> {
    let path = workspace_config_path(orbit_dir);
    let config = read_workspace_config(orbit_dir).map_err(|err| match err {
        OrbitError::InvalidInput(message) => OrbitError::InvalidInput(format!(
            "{message} (expected key `workspace_id` in {})",
            path.display()
        )),
        other => other,
    })?;
    Ok(config.workspace_id)
}

pub fn read_workspace_config_optional(
    orbit_dir: &Path,
) -> Result<Option<WorkspaceConfig>, OrbitError> {
    let path = workspace_config_path(orbit_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(OrbitError::Io(err.to_string())),
    };
    let doc: WorkspaceConfigDoc = serde_yaml::from_str(&raw).map_err(|e| {
        OrbitError::InvalidInput(format!(
            "invalid workspace config '{}': {e}",
            path.display()
        ))
    })?;
    validate_workspace_config_doc(doc).map(Some)
}

pub fn write_workspace_config(
    orbit_dir: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OrbitError> {
    let workspace_id = validate_workspace_id(&config.workspace_id)?;
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            config.schema_version
        )));
    }

    let doc = WorkspaceConfigDoc {
        schema_version: CONFIG_SCHEMA_VERSION,
        workspace_id,
    };
    let content = serde_yaml::to_string(&doc).map_err(|e| OrbitError::Store(e.to_string()))?;
    atomic_write_text(&workspace_config_path(orbit_dir), &content)
        .map_err(|e| OrbitError::Io(e.to_string()))
}

pub fn assign_workspace_id(slug_source: &str, path: &Path) -> String {
    workspace_id_candidate(&sanitize_slug(slug_source), path, 0)
}

fn validate_workspace_config_doc(doc: WorkspaceConfigDoc) -> Result<WorkspaceConfig, OrbitError> {
    if doc.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "unsupported workspace config schema_version {}",
            doc.schema_version
        )));
    }
    Ok(WorkspaceConfig {
        schema_version: doc.schema_version,
        workspace_id: validate_workspace_id(&doc.workspace_id)?,
    })
}
