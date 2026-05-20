use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde::Deserialize;

use crate::paths;
use crate::workspace_registry;

/// Returns the global orbit root at `~/.orbit/`.
pub(crate) fn resolve_global_root() -> Result<PathBuf, OrbitError> {
    workspace_registry::global_orbit_dir()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedOrbitRoots {
    pub(crate) shared_root: PathBuf,
    pub(crate) local_root: PathBuf,
}

impl ResolvedOrbitRoots {
    fn pinned(root: PathBuf) -> Self {
        Self {
            shared_root: root.clone(),
            local_root: root,
        }
    }

    fn new(shared_root: PathBuf, local_root: PathBuf) -> Self {
        Self {
            shared_root,
            local_root,
        }
    }
}

/// Resolves the `.orbit` shared and local roots using the full resolution chain.
///
/// Linked git worktrees intentionally keep `shared_root` pointed at the main
/// checkout's `.orbit` before local walk-up discovery, so existing task state
/// cannot diverge per worktree. They also expose a separate `local_root` at the
/// current linked checkout's `.orbit` for later per-worktree artifacts.
/// Explicit `--root` and `ORBIT_ROOT` still take precedence over this automatic
/// worktree resolution and pin both roots. A worktree-local `.orbit/` is not
/// consumed by any existing store in this phase.
///
/// Resolution order:
/// 1. `--root` flag (escape hatch)
/// 2. `ORBIT_ROOT` env (escape hatch)
/// 3. Linked git worktree's main checkout `.orbit/` as `shared_root`
/// 4. `path_overrides` in global registry (longest prefix match from cwd)
/// 5. Walk up from cwd to find first workspace `.orbit/` directory, skipping
///    the global home `.orbit/`
/// 6. Legacy: git repo root (for repos without `.orbit/` directory yet),
///    skipping the global home `.orbit/`
/// 7. Fallback: `<cwd>/.orbit`, refusing if it would resolve to the global
///    home `.orbit/`
pub(crate) fn resolve_initialize_roots(
    cwd: &Path,
    root_override: Option<&Path>,
) -> Result<ResolvedOrbitRoots, OrbitError> {
    resolve_roots(cwd, root_override, ExplicitRootMode::RequireInitialized)
}

/// Resolves `.orbit` roots for commands that are allowed to create them.
pub(crate) fn resolve_bootstrap_roots(
    cwd: &Path,
    root_override: Option<&Path>,
) -> Result<ResolvedOrbitRoots, OrbitError> {
    resolve_roots(cwd, root_override, ExplicitRootMode::AllowUninitialized)
}

/// Core implementation for `.orbit` workspace root discovery.
///
/// Explicit roots from `--root` and `ORBIT_ROOT` win first. Linked git
/// worktrees then resolve shared state through the main checkout while exposing
/// the linked checkout's local `.orbit/`, before registry overrides and legacy
/// walk-up run.
fn resolve_roots(
    cwd: &Path,
    root_override: Option<&Path>,
    explicit_root_mode: ExplicitRootMode,
) -> Result<ResolvedOrbitRoots, OrbitError> {
    // 1. --root flag (escape hatch)
    if let Some(root) = root_override {
        let root =
            resolve_explicit_root_path_value(&root.to_string_lossy(), cwd, explicit_root_mode)?;
        return Ok(log_resolved_roots(
            cwd,
            "explicit_root",
            ResolvedOrbitRoots::pinned(root),
        ));
    }

    // 2. ORBIT_ROOT env (escape hatch)
    if let Ok(explicit) = std::env::var("ORBIT_ROOT")
        && !explicit.trim().is_empty()
    {
        let root = resolve_explicit_root_path_value(&explicit, cwd, explicit_root_mode)?;
        return Ok(log_resolved_roots(
            cwd,
            "orbit_root_env",
            ResolvedOrbitRoots::pinned(root),
        ));
    }

    // 3. Linked git worktree's main checkout .orbit/ as shared_root
    if let Some(orbit_dir) = find_main_worktree_orbit_dir(cwd) {
        let shared_root = resolve_orbit_dir_candidate(&orbit_dir)?;
        let local_root = local_worktree_orbit_dir(cwd);
        return Ok(log_resolved_roots(
            cwd,
            "git_worktree_main",
            ResolvedOrbitRoots::new(shared_root, local_root),
        ));
    }

    // 4. path_overrides in global registry (longest prefix match)
    if let Some(ws) = resolve_from_path_override(cwd) {
        return Ok(log_resolved_roots(
            cwd,
            "path_override",
            ResolvedOrbitRoots::pinned(ws),
        ));
    }

    // 5. Walk up from cwd to find first workspace .orbit/ directory
    if let Some(orbit_dir) = find_orbit_dir_walk_up(cwd) {
        let root = resolve_orbit_dir_candidate(&orbit_dir)?;
        return Ok(log_resolved_roots(
            cwd,
            "walk_up",
            ResolvedOrbitRoots::pinned(root),
        ));
    }

    // 6. Legacy: git repo root (for repos without .orbit/ directory yet).
    //    Skip when the candidate equals the global $HOME/.orbit — that happens
    //    when $HOME is itself a git repo (e.g. yadm/chezmoi/vcsh dotfile
    //    managers), and adopting the global root as a workspace would silently
    //    corrupt user state.
    if let Some(repo_root) = paths::find_git_repo_root(cwd) {
        let candidate = repo_root.join(".orbit");
        if !is_global_orbit_dir(&candidate) {
            return Ok(log_resolved_roots(
                cwd,
                "git_repo_root",
                ResolvedOrbitRoots::pinned(candidate),
            ));
        }
    }

    // 7. Fallback: <cwd>/.orbit, but never the global $HOME/.orbit.
    let cwd_root = paths::cwd_orbit_root(cwd);
    if is_global_orbit_dir(&cwd_root) {
        return Err(OrbitError::InvalidInput(format!(
            "{} is the global Orbit root, not a workspace; run `orbit workspace init` from inside a project directory or pass `--root <path/to/.orbit>`",
            cwd_root.display()
        )));
    }
    Ok(log_resolved_roots(
        cwd,
        "cwd_fallback",
        ResolvedOrbitRoots::pinned(cwd_root),
    ))
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ExplicitRootMode {
    AllowUninitialized,
    RequireInitialized,
}

/// Checks path_overrides in the global registry for a matching workspace.
fn resolve_from_path_override(cwd: &Path) -> Option<PathBuf> {
    let registry = workspace_registry::load_registry().ok()?;
    let ws = workspace_registry::find_workspace_by_path(&registry, cwd)?;
    Some(ws.orbit_dir.clone())
}

fn find_main_worktree_orbit_dir(cwd: &Path) -> Option<PathBuf> {
    Some(paths::find_git_main_worktree_root(cwd)?.join(".orbit"))
}

fn local_worktree_orbit_dir(cwd: &Path) -> PathBuf {
    let worktree_root = paths::find_git_worktree_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    paths::normalize_path_components(&worktree_root.join(".orbit"))
}

/// Walks up the directory tree from `start` looking for the first workspace
/// `.orbit/` directory.
///
/// The user's global `$HOME/.orbit` is not a workspace root. Without this guard,
/// `orbit workspace init` in a repo under `$HOME` with no local `.orbit/` would
/// discover the global root before the git-repo bootstrap fallback and then
/// write workspace state into `$HOME/.orbit`.
fn find_orbit_dir_walk_up(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        let candidate = current.join(".orbit");
        if candidate.is_dir() && !is_global_orbit_dir(&candidate) {
            return Some(candidate);
        }
        current = current.parent()?;
    }
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

fn resolve_orbit_dir_candidate(orbit_dir: &Path) -> Result<PathBuf, OrbitError> {
    let config_path = orbit_dir.join("config.toml");
    if config_path.exists()
        && let Some(configured_root) = configured_root_from_config(&config_path)?
    {
        return Ok(configured_root);
    }
    Ok(orbit_dir.to_path_buf())
}

fn log_resolved_roots(
    cwd: &Path,
    source: &'static str,
    roots: ResolvedOrbitRoots,
) -> ResolvedOrbitRoots {
    tracing::debug!(
        source,
        cwd = %cwd.display(),
        shared_root = %roots.shared_root.display(),
        local_root = %roots.local_root.display(),
        "resolved Orbit roots"
    );
    roots
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

fn resolve_explicit_root_path_value(
    raw: &str,
    base_dir: &Path,
    mode: ExplicitRootMode,
) -> Result<PathBuf, OrbitError> {
    let root = resolve_root_path_value(raw, base_dir)?;
    match mode {
        ExplicitRootMode::AllowUninitialized => Ok(root),
        ExplicitRootMode::RequireInitialized => resolve_initialized_root(root),
    }
}

fn resolve_initialized_root(root: PathBuf) -> Result<PathBuf, OrbitError> {
    let child_orbit = root.join(".orbit");
    if is_initialized_orbit_root(&child_orbit) {
        return Ok(child_orbit);
    }

    if is_initialized_orbit_root(&root) {
        return Ok(root);
    }

    Err(OrbitError::InvalidInput(format!(
        "{} is not an Orbit workspace; run `orbit workspace init` first or pass `--root <path/to/.orbit>`",
        root.display()
    )))
}

fn is_initialized_orbit_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    if path.join("config.toml").is_file() {
        return true;
    }

    path.join("resources").is_dir() && path.join("tasks").is_dir() && path.join("state").is_dir()
}

fn resolve_root_path_value(raw: &str, base_dir: &Path) -> Result<PathBuf, OrbitError> {
    paths::resolve_path_value(raw, base_dir, "root path")
}

/// Like [`resolve_initialize_roots`] but never falls through to the
/// `<cwd>/.orbit` bootstrap fallback. Returns `Ok(None)` when no initialized
/// workspace is discovered anywhere in the chain.
///
/// Explicit roots (`--root`, `ORBIT_ROOT`) keep their `RequireInitialized`
/// semantics: pointing at an uninitialized path is still a hard error, since
/// the user explicitly asked for that root.
pub(crate) fn try_resolve_initialized_roots(
    cwd: &Path,
    root_override: Option<&Path>,
) -> Result<Option<ResolvedOrbitRoots>, OrbitError> {
    if let Some(root) = root_override {
        let root = resolve_explicit_root_path_value(
            &root.to_string_lossy(),
            cwd,
            ExplicitRootMode::RequireInitialized,
        )?;
        return Ok(Some(log_resolved_roots(
            cwd,
            "explicit_root",
            ResolvedOrbitRoots::pinned(root),
        )));
    }

    if let Ok(explicit) = std::env::var("ORBIT_ROOT")
        && !explicit.trim().is_empty()
    {
        let root =
            resolve_explicit_root_path_value(&explicit, cwd, ExplicitRootMode::RequireInitialized)?;
        return Ok(Some(log_resolved_roots(
            cwd,
            "orbit_root_env",
            ResolvedOrbitRoots::pinned(root),
        )));
    }

    if let Some(orbit_dir) = find_main_worktree_orbit_dir(cwd)
        && is_initialized_orbit_root(&orbit_dir)
    {
        let shared_root = resolve_orbit_dir_candidate(&orbit_dir)?;
        let local_root = local_worktree_orbit_dir(cwd);
        return Ok(Some(log_resolved_roots(
            cwd,
            "git_worktree_main",
            ResolvedOrbitRoots::new(shared_root, local_root),
        )));
    }

    if let Some(ws) = resolve_from_path_override(cwd)
        && is_initialized_orbit_root(&ws)
    {
        return Ok(Some(log_resolved_roots(
            cwd,
            "path_override",
            ResolvedOrbitRoots::pinned(ws),
        )));
    }

    if let Some(orbit_dir) = find_orbit_dir_walk_up(cwd)
        && is_initialized_orbit_root(&orbit_dir)
    {
        let root = resolve_orbit_dir_candidate(&orbit_dir)?;
        return Ok(Some(log_resolved_roots(
            cwd,
            "walk_up",
            ResolvedOrbitRoots::pinned(root),
        )));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn explicit_root_with_initialized_child_orbit_resolves_to_child() {
        let repo = tempdir().expect("repo tempdir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);

        let resolved =
            resolve_initialize_roots(repo.path(), Some(repo.path())).expect("resolve root");

        assert_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn explicit_root_prefers_initialized_child_orbit_over_polluted_repo_root() {
        let repo = tempdir().expect("repo tempdir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);
        fs::write(repo.path().join("config.toml"), "polluted = true\n")
            .expect("write root pollution");

        let resolved =
            resolve_initialize_roots(repo.path(), Some(repo.path())).expect("resolve root");

        assert_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn explicit_root_with_uninitialized_directory_returns_invalid_input_without_layout() {
        let parent = tempdir().expect("parent tempdir");
        let root = parent.path().join("not-an-orbit-root");
        fs::create_dir_all(&root).expect("create uninitialized root");

        let err = resolve_initialize_roots(parent.path(), Some(&root))
            .expect_err("uninitialized root should fail");

        assert!(matches!(
            err,
            OrbitError::InvalidInput(message) if message.contains("not an Orbit workspace")
        ));
        assert!(!root.join(".orbit").exists());
        assert!(!root.join("resources").exists());
        assert!(!root.join("tasks").exists());
        assert!(!root.join("state").exists());
    }

    #[test]
    fn explicit_root_with_initialized_orbit_root_resolves_as_is() {
        let repo = tempdir().expect("repo tempdir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);

        let resolved =
            resolve_initialize_roots(repo.path(), Some(&orbit_root)).expect("resolve root");

        assert_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn bootstrap_root_allows_uninitialized_path_without_creating_it() {
        let parent = tempdir().expect("parent tempdir");
        let root = parent.path().join("new-orbit-root");

        let resolved = resolve_bootstrap_roots(parent.path(), Some(&root)).expect("resolve root");

        assert_pinned_roots(&resolved, &root);
        assert!(!root.exists());
    }

    #[test]
    fn explicit_root_precedes_env_and_worktree_resolution() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        let explicit_repo = tempdir().expect("explicit repo tempdir");
        let env_repo = tempdir().expect("env repo tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());
        seed_initialized_workspace_root(&main_repo.path().join(".orbit"));
        seed_initialized_workspace_root(&explicit_repo.path().join(".orbit"));
        seed_initialized_workspace_root(&env_repo.path().join(".orbit"));
        let _env = EnvVarGuard::set("ORBIT_ROOT", env_repo.path().as_os_str().to_os_string());

        let resolved = resolve_initialize_roots(worktree.path(), Some(explicit_repo.path()))
            .expect("resolve explicit root");

        assert_pinned_roots(&resolved, &explicit_repo.path().join(".orbit"));
    }

    #[test]
    fn env_root_precedes_worktree_resolution() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        let env_repo = tempdir().expect("env repo tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());
        seed_initialized_workspace_root(&main_repo.path().join(".orbit"));
        seed_initialized_workspace_root(&env_repo.path().join(".orbit"));
        let _env = EnvVarGuard::set("ORBIT_ROOT", env_repo.path().as_os_str().to_os_string());

        let resolved = resolve_initialize_roots(worktree.path(), None).expect("resolve env root");

        assert_pinned_roots(&resolved, &env_repo.path().join(".orbit"));
    }

    #[test]
    fn worktree_main_orbit_precedes_worktree_local_orbit() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());
        let main_orbit = main_repo.path().join(".orbit");
        let worktree_orbit = worktree.path().join(".orbit");
        seed_initialized_workspace_root(&main_orbit);
        seed_initialized_workspace_root(&worktree_orbit);

        let resolved =
            resolve_initialize_roots(worktree.path(), None).expect("resolve worktree root");

        assert_roots(&resolved, &main_orbit, &worktree_orbit);
    }

    #[test]
    fn worktree_without_orbit_uses_main_repo_legacy_orbit_path() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());

        let resolved =
            resolve_bootstrap_roots(worktree.path(), None).expect("resolve worktree root");

        assert_roots(
            &resolved,
            &main_repo.path().join(".orbit"),
            &worktree.path().join(".orbit"),
        );
        assert!(!resolved.shared_root.exists());
        assert!(!worktree.path().join(".orbit").exists());
    }

    #[test]
    fn non_worktree_walk_up_behavior_is_preserved() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let repo = tempdir().expect("repo tempdir");
        let nested = repo.path().join("a").join("b");
        fs::create_dir_all(&nested).expect("create nested dir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);

        let resolved = resolve_initialize_roots(&nested, None).expect("resolve walk-up root");

        assert_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn bootstrap_rejects_home_when_cwd_is_home_with_global_orbit_and_no_git() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let global_orbit = home.path().join(".orbit");
        seed_initialized_workspace_root(&global_orbit);
        let _home = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());
        let _orbit_root = EnvVarGuard::remove("ORBIT_ROOT");

        let err = resolve_bootstrap_roots(home.path(), None)
            .expect_err("bootstrap should refuse to adopt the global root as a workspace");

        assert!(matches!(
            err,
            OrbitError::InvalidInput(message) if message.contains("global Orbit root")
        ));
    }

    #[test]
    fn bootstrap_rejects_home_when_home_itself_is_a_git_repo() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        fs::create_dir_all(home.path().join(".git")).expect("seed home as git repo");
        let global_orbit = home.path().join(".orbit");
        seed_initialized_workspace_root(&global_orbit);
        let _home = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());
        let _orbit_root = EnvVarGuard::remove("ORBIT_ROOT");

        let err = resolve_bootstrap_roots(home.path(), None)
            .expect_err("bootstrap should refuse $HOME/.orbit via git_repo_root + cwd_fallback");

        assert!(matches!(
            err,
            OrbitError::InvalidInput(message) if message.contains("global Orbit root")
        ));
    }

    #[test]
    fn bootstrap_ignores_home_global_orbit_when_repo_has_no_workspace_orbit() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let repo = home.path().join("work").join("repo");
        fs::create_dir_all(repo.join(".git")).expect("create repo git dir");
        let global_orbit = home.path().join(".orbit");
        seed_initialized_workspace_root(&global_orbit);
        let _home = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());
        let _orbit_root = EnvVarGuard::remove("ORBIT_ROOT");

        let resolved = resolve_bootstrap_roots(&repo, None).expect("resolve bootstrap root");

        assert_pinned_roots(&resolved, &repo.join(".orbit"));
        assert_ne!(resolved.shared_root, global_orbit);
    }

    #[test]
    fn try_resolve_returns_none_outside_orbit_workspace() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let nowhere = tempdir().expect("nowhere tempdir");

        let resolved = try_resolve_initialized_roots(nowhere.path(), None)
            .expect("try_resolve completes without error");

        assert!(resolved.is_none());
        assert!(!nowhere.path().join(".orbit").exists());
    }

    #[test]
    fn try_resolve_finds_initialized_workspace_via_walk_up() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let repo = tempdir().expect("repo tempdir");
        let nested = repo.path().join("a").join("b");
        fs::create_dir_all(&nested).expect("create nested dir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);

        let resolved = try_resolve_initialized_roots(&nested, None)
            .expect("try_resolve completes without error");

        assert_optional_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn try_resolve_finds_main_worktree_orbit_for_linked_worktree() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());
        let main_orbit = main_repo.path().join(".orbit");
        seed_initialized_workspace_root(&main_orbit);

        let resolved = try_resolve_initialized_roots(worktree.path(), None)
            .expect("try_resolve completes without error");

        assert_optional_roots(&resolved, &main_orbit, &worktree.path().join(".orbit"));
    }

    #[test]
    fn try_resolve_returns_none_when_main_worktree_orbit_is_uninitialized() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let main_repo = tempdir().expect("main repo tempdir");
        let worktree = tempdir().expect("worktree tempdir");
        seed_fake_git_worktree(main_repo.path(), worktree.path());
        // No `.orbit/` exists at all — main worktree resolution finds the
        // path but it's uninitialized, so try_resolve falls through.

        let resolved = try_resolve_initialized_roots(worktree.path(), None)
            .expect("try_resolve completes without error");

        assert!(resolved.is_none());
        assert!(!main_repo.path().join(".orbit").exists());
        assert!(!worktree.path().join(".orbit").exists());
    }

    #[test]
    fn try_resolve_honors_initialized_root_override() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let repo = tempdir().expect("repo tempdir");
        let orbit_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&orbit_root);
        let elsewhere = tempdir().expect("elsewhere tempdir");

        let resolved = try_resolve_initialized_roots(elsewhere.path(), Some(repo.path()))
            .expect("try_resolve completes without error");

        assert_optional_pinned_roots(&resolved, &orbit_root);
    }

    #[test]
    fn try_resolve_rejects_uninitialized_root_override() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let _env = EnvVarGuard::remove("ORBIT_ROOT");
        let parent = tempdir().expect("parent tempdir");
        let bogus = parent.path().join("not-an-orbit-root");
        fs::create_dir_all(&bogus).expect("create bogus dir");

        let err = try_resolve_initialized_roots(parent.path(), Some(&bogus))
            .expect_err("uninitialized override should error");

        assert!(matches!(
            err,
            OrbitError::InvalidInput(message) if message.contains("not an Orbit workspace")
        ));
        assert!(!bogus.join(".orbit").exists());
    }

    fn seed_initialized_workspace_root(path: &Path) {
        fs::create_dir_all(path.join("resources")).expect("create resources");
        fs::create_dir_all(path.join("tasks")).expect("create tasks");
        fs::create_dir_all(path.join("state")).expect("create state");
    }

    fn assert_pinned_roots(roots: &ResolvedOrbitRoots, root: &Path) {
        assert_roots(roots, root, root);
    }

    fn assert_roots(roots: &ResolvedOrbitRoots, shared_root: &Path, local_root: &Path) {
        assert_eq!(roots.shared_root, shared_root);
        assert_eq!(roots.local_root, local_root);
    }

    fn assert_optional_pinned_roots(roots: &Option<ResolvedOrbitRoots>, root: &Path) {
        assert_optional_roots(roots, root, root);
    }

    fn assert_optional_roots(
        roots: &Option<ResolvedOrbitRoots>,
        shared_root: &Path,
        local_root: &Path,
    ) {
        let roots = roots.as_ref().expect("expected resolved roots");
        assert_roots(roots, shared_root, local_root);
    }

    fn seed_fake_git_worktree(main_repo: &Path, worktree: &Path) {
        let worktree_git_dir = main_repo.join(".git").join("worktrees").join("orbit-test");
        fs::create_dir_all(&worktree_git_dir).expect("create fake worktree git dir");
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", worktree_git_dir.display()),
        )
        .expect("write worktree gitfile");
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: OsString) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }
}
