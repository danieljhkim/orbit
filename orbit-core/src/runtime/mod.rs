pub mod audit;
pub mod builder;
pub mod event_bus;
pub mod mutation;
pub mod pipeline;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_policy::PolicyEngine;
use orbit_types::{Audit, IdentityRole, Job, OrbitError, OrbitEvent, ResolvedIdentity};
use serde::Deserialize;
use serde_json::Value;

use crate::OrbitContext;
use crate::command::init::ensure_orbit_root_initialized;
use crate::identity_catalog::compile_identity_block;
use crate::paths;

#[derive(Clone)]
pub struct OrbitRuntime {
    pub(crate) context: OrbitContext,
    pub event_log: event_bus::EventLog,
}

impl OrbitRuntime {
    pub fn initialize() -> Result<Self, OrbitError> {
        Self::initialize_with_root_override(None)
    }

    pub fn initialize_with_root_override(root_override: Option<&Path>) -> Result<Self, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let orbit_home = Self::default_data_root();
        let data_root = resolve_initialize_data_root(&cwd, root_override, &orbit_home)?;
        if should_bootstrap_orbit_home(&data_root, &orbit_home) {
            ensure_orbit_root_initialized(&orbit_home)?;
        }
        Self::from_data_root(&data_root)
    }

    pub fn from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        let orbit_home = Self::orbit_home_root();
        Ok(Self {
            context: builder::build_context_from_data_root(data_root, &orbit_home)?,
            event_log: event_bus::EventLog::default(),
        })
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        Ok(Self {
            context: builder::build_context_in_memory()?,
            event_log: event_bus::EventLog::default(),
        })
    }

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.context.policy = policy;
        self
    }

    pub fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        let events = self.event_log.snapshot();
        let audits = events
            .into_iter()
            .enumerate()
            .map(|(idx, event)| orbit_event_to_audit((idx + 1) as i64, event))
            .rev()
            .take(limit)
            .collect();
        Ok(audits)
    }

    pub fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.context.job_store.get_job(job_id)
    }

    pub fn execution_env_config(&self) -> (bool, Vec<String>) {
        (
            self.context.execution_env_policy.inherit(),
            self.context.execution_env_policy.pass().to_vec(),
        )
    }

    pub fn codex_execution_config(&self) -> (String, Option<String>) {
        (
            self.context.codex_execution_policy.sandbox().to_string(),
            self.context
                .codex_execution_policy
                .approval_policy()
                .map(ToString::to_string),
        )
    }

    pub fn data_root(&self) -> PathBuf {
        self.context.data_root.clone()
    }

    pub fn orbit_home(&self) -> PathBuf {
        self.context.orbit_home.clone()
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_root().join("config.toml")
    }

    pub fn persistence_config_json(&self) -> Value {
        self.context.persistence.as_json_value()
    }

    pub fn task_approval_required_for_agent(&self) -> bool {
        self.context.task_approval_required_for_agent
    }

    pub fn task_delegate_approval(&self) -> bool {
        self.context.task_delegate_approval
    }

    pub fn identity_root(&self) -> PathBuf {
        self.context.identity_catalog.root().to_path_buf()
    }

    pub fn identity_role_overrides(&self) -> std::collections::BTreeMap<String, String> {
        self.context
            .identity_catalog
            .role_overrides()
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect()
    }

    pub fn list_identities(&self) -> Result<Vec<ResolvedIdentity>, OrbitError> {
        self.context.identity_catalog.list_filtered(None)
    }

    pub fn list_identities_filtered(
        &self,
        role: Option<IdentityRole>,
    ) -> Result<Vec<ResolvedIdentity>, OrbitError> {
        self.context.identity_catalog.list_filtered(role)
    }

    pub fn show_identity(&self, identity_id: &str) -> Result<ResolvedIdentity, OrbitError> {
        self.context.identity_catalog.resolve(identity_id)
    }

    pub fn resolve_identity(&self, identity_id: &str) -> Result<ResolvedIdentity, OrbitError> {
        self.context.identity_catalog.resolve(identity_id)
    }

    pub fn compile_identity_block(&self, identity: &ResolvedIdentity) -> String {
        compile_identity_block(identity)
    }

    pub fn run_jobs(&self) -> Result<usize, OrbitError> {
        self.run_due_jobs(Utc::now())
    }

    pub fn default_data_root() -> PathBuf {
        Self::orbit_home_root()
    }

    pub fn orbit_home_root() -> PathBuf {
        paths::orbit_home_root()
    }
}

fn orbit_event_to_audit(id: i64, event: OrbitEvent) -> Audit {
    let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();

    Audit {
        id,
        event_type: event_type.clone(),
        payload,
        message: event_type,
        created_at: Utc::now(),
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

pub(crate) fn resolve_initialize_data_root(
    cwd: &Path,
    root_override: Option<&Path>,
    orbit_home: &Path,
) -> Result<PathBuf, OrbitError> {
    if let Some(root) = root_override {
        return resolve_root_path_value(&root.to_string_lossy(), cwd);
    }

    if let Ok(explicit) = std::env::var("ORBIT_ROOT")
        && !explicit.trim().is_empty()
    {
        return resolve_root_path_value(&explicit, cwd);
    }

    if let Some(repo_root) = find_git_repo_root(cwd) {
        let repo_orbit_root = repo_root.join(".orbit");
        let repo_config = repo_orbit_root.join("config.toml");
        if repo_config.exists() {
            if let Some(configured_root) = configured_root_from_config(&repo_config)? {
                return Ok(configured_root);
            }
            return Ok(repo_orbit_root);
        }
    }

    let home_config = orbit_home.join("config.toml");
    if home_config.exists() {
        if let Some(configured_root) = configured_root_from_config(&home_config)? {
            return Ok(configured_root);
        }
        return Ok(orbit_home.to_path_buf());
    }

    Ok(orbit_home.to_path_buf())
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

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    paths::find_git_repo_root(start)
}

fn should_bootstrap_orbit_home(data_root: &Path, orbit_home: &Path) -> bool {
    data_root == orbit_home
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::{resolve_initialize_data_root, should_bootstrap_orbit_home};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn cli_root_override_has_highest_precedence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("cli-root");
        let orbit_home = dir.path().join("home").join(".orbit");
        let chosen = resolve_initialize_data_root(cwd, Some(explicit.as_path()), &orbit_home)
            .expect("resolve");
        assert_eq!(chosen, explicit);
    }

    #[test]
    fn orbit_root_env_overrides_config_roots() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("env-root");
        let orbit_home = dir.path().join("home").join(".orbit");

        let previous = std::env::var("ORBIT_ROOT").ok();
        // SAFETY: test runs in isolation; no other thread reads this var concurrently.
        unsafe { std::env::set_var("ORBIT_ROOT", &explicit) };
        let chosen = resolve_initialize_data_root(cwd, None, &orbit_home).expect("resolve");
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

        let orbit_home = dir.path().join("home").join(".orbit");
        std::fs::create_dir_all(&orbit_home).expect("home orbit");
        std::fs::write(
            orbit_home.join("config.toml"),
            "root = \"./home-root\"\n[task.approval]\nrequired_for_agent=false\n",
        )
        .expect("write home config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None, &orbit_home).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo_orbit.join("repo-root"));
    }

    #[test]
    fn home_config_root_used_when_repo_config_missing() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let orbit_home = dir.path().join("home").join(".orbit");
        std::fs::create_dir_all(&orbit_home).expect("home orbit");
        std::fs::write(
            orbit_home.join("config.toml"),
            "root = \"./configured-home-root\"\n",
        )
        .expect("write home config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(cwd, None, &orbit_home).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, orbit_home.join("configured-home-root"));
    }

    #[test]
    fn home_config_root_used_when_inside_git_repo_without_repo_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let orbit_home = dir.path().join("home").join(".orbit");
        std::fs::create_dir_all(&orbit_home).expect("home orbit");
        std::fs::write(
            orbit_home.join("config.toml"),
            "root = \"./configured-home-root\"\n",
        )
        .expect("write home config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None, &orbit_home).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, orbit_home.join("configured-home-root"));
    }

    #[test]
    fn orbit_home_fallback_used_when_no_override_or_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let orbit_home = dir.path().join("home").join(".orbit");
        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(cwd, None, &orbit_home).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, orbit_home);
    }

    #[test]
    fn configured_root_normalizes_curdir_segments() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let orbit_home = dir.path().join("home").join(".orbit");
        std::fs::create_dir_all(&orbit_home).expect("home orbit");
        std::fs::write(orbit_home.join("config.toml"), "root = \".\"\n").expect("write config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            // SAFETY: test runs in isolation; no other thread reads this var concurrently.
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(dir.path(), None, &orbit_home).expect("resolve");
        if let Some(value) = previous {
            // SAFETY: test restores previous env var for isolation.
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, orbit_home);
    }

    #[test]
    fn bootstrap_home_only_when_home_is_selected_data_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let orbit_home = dir.path().join("home").join(".orbit");
        let repo_orbit = dir.path().join("repo").join(".orbit");

        assert!(should_bootstrap_orbit_home(&orbit_home, &orbit_home));
        assert!(!should_bootstrap_orbit_home(&repo_orbit, &orbit_home));
    }
}
