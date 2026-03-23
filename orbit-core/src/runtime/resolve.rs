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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::{find_orbit_dir_walk_up, resolve_initialize_data_root};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn cli_root_override_has_highest_precedence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("cli-root");
        let chosen = resolve_initialize_data_root(cwd, Some(explicit.as_path())).expect("resolve");
        assert_eq!(chosen, explicit);
    }

    #[test]
    fn orbit_root_env_overrides_config_roots() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("env-root");

        let previous = std::env::var("ORBIT_ROOT").ok();
        unsafe { std::env::set_var("ORBIT_ROOT", &explicit) };
        let chosen = resolve_initialize_data_root(cwd, None).expect("resolve");
        match previous {
            Some(value) => unsafe { std::env::set_var("ORBIT_ROOT", value) },
            None => unsafe { std::env::remove_var("ORBIT_ROOT") },
        }

        assert_eq!(chosen, explicit);
    }

    #[test]
    fn repo_config_root_has_precedence_over_home_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let repo_orbit = repo.join(".orbit");
        std::fs::create_dir_all(&repo_orbit).expect("repo orbit");
        std::fs::write(
            repo_orbit.join("config.toml"),
            "root = \"./repo-root\"\n[task.approval]\nrequired_for_agent=true\n",
        )
        .expect("write repo config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo_orbit.join("repo-root"));
    }

    #[test]
    fn repo_root_used_when_inside_git_repo_without_repo_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo.join(".orbit"));
    }

    #[test]
    fn cwd_root_used_when_outside_git_repo_without_override_or_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, cwd.join(".orbit"));
    }

    #[test]
    fn repo_root_used_even_when_repo_orbit_directory_is_absent() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo.join(".orbit"));
    }

    #[test]
    fn configured_root_normalizes_curdir_segments() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        let repo_orbit = repo.join(".orbit");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(&repo_orbit).expect("repo orbit");
        std::fs::write(repo_orbit.join("config.toml"), "root = \".\"\n").expect("write config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo_orbit);
    }

    #[test]
    fn walk_up_finds_orbit_dir_above_cwd() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("project");
        let orbit_dir = project.join(".orbit");
        let cwd = project.join("src").join("deep");
        std::fs::create_dir_all(&orbit_dir).expect("create .orbit");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, orbit_dir);
    }

    #[test]
    fn find_orbit_dir_walk_up_returns_none_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("no-orbit");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        assert!(find_orbit_dir_walk_up(&cwd).is_none());
    }
}
