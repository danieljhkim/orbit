use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::OrbitError;
use serde::Deserialize;

use crate::paths;
use crate::workspace_registry;

/// Returns the global orbit root at `~/.orbit/`.
pub(crate) fn resolve_global_root() -> Result<PathBuf, OrbitError> {
    workspace_registry::global_orbit_dir()
}

/// Resolves the `.orbit` data root using the full resolution chain.
///
/// Resolution order:
/// 1. `--root` flag (escape hatch)
/// 2. `ORBIT_ROOT` env (escape hatch)
/// 3. `path_overrides` in global registry (longest prefix match from cwd)
/// 4. Walk up from cwd to find first `.orbit/` directory
/// 5. Legacy: git repo root (for repos without `.orbit/` directory yet)
/// 6. Fallback: `<cwd>/.orbit`
pub(crate) fn resolve_initialize_data_root(
    cwd: &Path,
    root_override: Option<&Path>,
) -> Result<PathBuf, OrbitError> {
    // 1. --root flag (escape hatch)
    if let Some(root) = root_override {
        return resolve_root_path_value(&root.to_string_lossy(), cwd);
    }

    // 2. ORBIT_ROOT env (escape hatch)
    if let Ok(explicit) = std::env::var("ORBIT_ROOT")
        && !explicit.trim().is_empty()
    {
        return resolve_root_path_value(&explicit, cwd);
    }

    // 3. path_overrides in global registry (longest prefix match)
    if let Some(ws) = resolve_from_path_override(cwd) {
        return Ok(ws);
    }

    // 4. Walk up from cwd to find first .orbit/ directory
    if let Some(orbit_dir) = find_orbit_dir_walk_up(cwd) {
        // Check if this .orbit has a config.toml with a root redirect
        let config_path = orbit_dir.join("config.toml");
        if config_path.exists()
            && let Some(configured_root) = configured_root_from_config(&config_path)?
        {
            return Ok(configured_root);
        }
        return Ok(orbit_dir);
    }

    // 5. Legacy: git repo root (for repos without .orbit/ directory yet)
    if let Some(repo_root) = paths::find_git_repo_root(cwd) {
        return Ok(repo_root.join(".orbit"));
    }

    // 6. Fallback: <cwd>/.orbit
    Ok(paths::cwd_orbit_root(cwd))
}

/// Checks path_overrides in the global registry for a matching workspace.
fn resolve_from_path_override(cwd: &Path) -> Option<PathBuf> {
    let registry = workspace_registry::load_registry().ok()?;
    let ws = workspace_registry::find_workspace_by_path(&registry, cwd)?;
    Some(ws.orbit_dir.clone())
}

/// Walks up the directory tree from `start` looking for the first `.orbit/` directory.
fn find_orbit_dir_walk_up(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        let candidate = current.join(".orbit");
        if candidate.is_dir() {
            return Some(candidate);
        }
        current = current.parent()?;
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RootField {
    String(String),
    Table { path: String },
}

#[derive(Debug, Deserialize)]
struct RootOnlyConfig {
    root: Option<RootField>,
}

fn configured_root_from_config(config_path: &Path) -> Result<Option<PathBuf>, OrbitError> {
    let raw = fs::read_to_string(config_path).map_err(|err| {
        OrbitError::Io(format!(
            "failed to read runtime config '{}': {err}",
            config_path.display()
        ))
    })?;
    let parsed = toml::from_str::<RootOnlyConfig>(&raw).map_err(|err| {
        OrbitError::InvalidInput(format!(
            "invalid runtime config '{}': {err}",
            config_path.display()
        ))
    })?;
    let Some(root_value) = parsed.root else {
        return Ok(None);
    };
    let root_value = match root_value {
        RootField::String(value) => value,
        RootField::Table { path } => path,
    };
    let base = config_path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "invalid config path without parent: {}",
            config_path.display()
        ))
    })?;
    Ok(Some(resolve_root_path_value(&root_value, base)?))
}

fn resolve_root_path_value(raw: &str, base_dir: &Path) -> Result<PathBuf, OrbitError> {
    paths::resolve_path_value(raw, base_dir, "root path")
}
