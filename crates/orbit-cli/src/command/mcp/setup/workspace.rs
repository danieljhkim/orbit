use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_cmd::registry_runtime::RegisteredRuntimeFactory;
use orbit_core::OrbitError;
use orbit_registry::workspace_registry;

#[derive(Debug, Clone)]
pub(super) struct WorkspaceLayout {
    pub(super) repo_root: PathBuf,
    pub(super) orbit_root: PathBuf,
}

pub(super) fn resolve_workspace_layout(
    root_override: Option<&Path>,
) -> Result<WorkspaceLayout, OrbitError> {
    if let Some(orbit_root) = root_override {
        return Ok(WorkspaceLayout {
            repo_root: orbit_root.parent().unwrap_or(orbit_root).to_path_buf(),
            orbit_root: orbit_root.to_path_buf(),
        });
    }

    let cwd = env::current_dir().map_err(|err| OrbitError::Io(err.to_string()))?;
    resolve_workspace_layout_for_cwd(&cwd)
}

pub(super) fn resolve_workspace_layout_for_cwd(cwd: &Path) -> Result<WorkspaceLayout, OrbitError> {
    if cwd.file_name().is_some_and(|name| name == ".orbit") && cwd.is_dir() {
        return Ok(WorkspaceLayout {
            repo_root: cwd.parent().unwrap_or(cwd).to_path_buf(),
            orbit_root: cwd.to_path_buf(),
        });
    }

    // Skip the user's global $HOME/.orbit during ancestor walk-up. It is the
    // global Orbit root, not a workspace, so adopting it would silently write
    // workspace-scope MCP configs to home-scope paths.
    let home = env_home_dir();
    for ancestor in cwd.ancestors() {
        let orbit_root = ancestor.join(".orbit");
        if orbit_root.is_dir() && !is_global_orbit_dir(&orbit_root) {
            return Ok(WorkspaceLayout {
                repo_root: ancestor.to_path_buf(),
                orbit_root,
            });
        }
        if home
            .as_deref()
            .is_some_and(|home| paths_equivalent(ancestor, home))
        {
            break;
        }
    }

    Err(OrbitError::InvalidInput(
        "current directory is not inside an initialized Orbit workspace; run `orbit workspace init` first or pass `--root <path/to/.orbit>`".to_string(),
    ))
}

/// The logical workspace ID this checkout is registered under on this machine,
/// or `None` when the machine registry does not know it.
///
/// This is what a generated integration binds its MCP server to. The logical
/// `ws_*` ID is used rather than the checkout path so a linked worktree — whose
/// own checkout identity may have diverged from the registration — still names
/// one workspace, and so a config that travels with the repo does not carry an
/// absolute path from the machine that wrote it.
///
/// An unregistered checkout is not an error here: the generated server simply
/// stays unbound, exactly as it was before a binding existed, and every
/// workspace-scoped call supplies its own selector.
pub(super) fn registered_workspace_id(repo_root: &Path) -> Option<String> {
    let global_root = workspace_registry::global_orbit_dir().ok()?;
    let selector = repo_root.to_str()?;
    RegisteredRuntimeFactory::resolve_workspace_selector(&global_root, selector)
        .ok()
        .map(|selected| selected.workspace.id)
}

fn is_global_orbit_dir(candidate: &Path) -> bool {
    let Ok(global) = workspace_registry::global_orbit_dir() else {
        return false;
    };
    paths_equivalent(candidate, &global)
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

pub(super) fn env_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}
