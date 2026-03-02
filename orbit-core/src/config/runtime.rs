use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::{IdentityRole, OrbitError};

use super::persistence::{PersistenceConfig, resolve_path};
use super::raw::{RawExecutionEnvConfig, RawIdentitySection, RawRuntimeConfig, RawTaskSection};

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_ENV_PASS: [&str; 3] = ["HOME", "PATH", "CODEX_HOME"];
const DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT: bool = false;
const DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL: bool = false;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval: TaskApprovalConfig,
    pub(crate) identity: IdentityConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let orbit_home = orbit_home_root();
        Self::default_for_roots(&orbit_home, &orbit_home)
    }
}

impl RuntimeConfig {
    pub(crate) fn default_for_roots(data_root: &Path, orbit_home: &Path) -> Self {
        Self {
            execution_env: ExecutionEnvPolicy::default(),
            persistence: PersistenceConfig::default_for_data_root(data_root),
            task_approval: TaskApprovalConfig::default(),
            identity: IdentityConfig::default_for_orbit_home(orbit_home),
        }
    }

    pub(crate) fn load_from_data_root(
        data_root: &Path,
        orbit_home: &Path,
    ) -> Result<Self, OrbitError> {
        let config_path = data_root.join("config.toml");
        if !config_path.exists() {
            return Ok(Self::default_for_roots(data_root, orbit_home));
        }

        let raw = fs::read_to_string(&config_path).map_err(|err| {
            OrbitError::Io(format!(
                "failed to read runtime config '{}': {err}",
                config_path.display()
            ))
        })?;
        let parsed = toml::from_str::<RawRuntimeConfig>(&raw).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid runtime config '{}': {err}",
                config_path.display()
            ))
        })?;

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.env),
            )?,
            persistence: PersistenceConfig::from_raw(data_root, &parsed)?,
            task_approval: TaskApprovalConfig::from_raw(parsed.task.as_ref())?,
            identity: IdentityConfig::from_raw(parsed.identity.as_ref(), data_root, orbit_home)?,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TaskApprovalConfig {
    pub(crate) required_for_agent: bool,
    pub(crate) delegate_approval: bool,
}

impl Default for TaskApprovalConfig {
    fn default() -> Self {
        Self {
            required_for_agent: DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT,
            delegate_approval: DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL,
        }
    }
}

impl TaskApprovalConfig {
    fn from_raw(raw: Option<&RawTaskSection>) -> Result<Self, OrbitError> {
        let required_for_agent = raw
            .and_then(|section| section.approval.as_ref())
            .and_then(|approval| approval.required_for_agent)
            .unwrap_or(DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT);
        let delegate_approval = raw
            .and_then(|section| section.approval.as_ref())
            .and_then(|approval| approval.delegate_approval)
            .unwrap_or(DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL);
        Ok(Self {
            required_for_agent,
            delegate_approval,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityConfig {
    pub(crate) root: PathBuf,
    pub(crate) role_overrides: BTreeMap<String, IdentityRole>,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self::default_for_orbit_home(&orbit_home_root())
    }
}

impl IdentityConfig {
    fn default_for_orbit_home(orbit_home: &Path) -> Self {
        Self {
            root: orbit_home.join("identities"),
            role_overrides: BTreeMap::new(),
        }
    }

    fn from_raw(
        raw: Option<&RawIdentitySection>,
        config_root: &Path,
        orbit_home: &Path,
    ) -> Result<Self, OrbitError> {
        let default = Self::default_for_orbit_home(orbit_home);
        let root = resolve_path(
            raw.and_then(|v| v.root.as_deref()),
            &default.root,
            config_root,
        )?;
        let mut role_overrides = BTreeMap::new();
        if let Some(roles) = raw.and_then(|v| v.roles.as_ref()) {
            for (identity, role_raw) in roles {
                let key = identity.trim();
                if key.is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "identity.roles keys must not be empty".to_string(),
                    ));
                }
                let role = role_raw.parse::<IdentityRole>().map_err(|e| {
                    OrbitError::InvalidInput(format!(
                        "identity.roles.{key} has invalid role '{role_raw}': {e}"
                    ))
                })?;
                role_overrides.insert(key.to_string(), role);
            }
        }
        Ok(Self {
            root,
            role_overrides,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionEnvPolicy {
    inherit: bool,
    pass: Vec<String>,
}

impl Default for ExecutionEnvPolicy {
    fn default() -> Self {
        Self {
            inherit: DEFAULT_ENV_INHERIT,
            pass: default_pass_list(),
        }
    }
}

impl ExecutionEnvPolicy {
    fn from_raw(raw: Option<RawExecutionEnvConfig>) -> Result<Self, OrbitError> {
        match raw {
            Some(raw) => {
                let inherit = raw.inherit.unwrap_or(DEFAULT_ENV_INHERIT);
                let pass = normalize_pass_list(raw.pass.unwrap_or_else(default_pass_list))?;
                Ok(Self { inherit, pass })
            }
            None => Ok(Self::default()),
        }
    }

    pub(crate) fn inherit(&self) -> bool {
        self.inherit
    }

    pub(crate) fn pass(&self) -> &[String] {
        &self.pass
    }

    pub(crate) fn hydrated_allowlist_env(&self) -> Vec<(String, String)> {
        self.pass
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect()
    }

    pub(crate) fn missing_required_for_provider(&self, provider: &str) -> Vec<String> {
        required_env_vars_for_provider(provider)
            .iter()
            .copied()
            .filter(|name| !self.is_required_var_available(name))
            .map(ToString::to_string)
            .collect()
    }

    fn is_required_var_available(&self, name: &str) -> bool {
        if self.inherit {
            return std::env::var(name).is_ok();
        }
        self.pass.iter().any(|candidate| candidate == name) && std::env::var(name).is_ok()
    }
}

fn orbit_home_root() -> PathBuf {
    home_dir()
        .map(|home| home.join(".orbit"))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".orbit")
        })
}

fn default_pass_list() -> Vec<String> {
    DEFAULT_ENV_PASS.iter().map(ToString::to_string).collect()
}

pub(super) fn normalize_pass_list(pass: Vec<String>) -> Result<Vec<String>, OrbitError> {
    let mut normalized = BTreeSet::new();
    for entry in pass {
        let value = entry.trim();
        if value.is_empty() {
            return Err(OrbitError::InvalidInput(
                "execution.env.pass must not contain empty variable names".to_string(),
            ));
        }
        if !is_valid_env_var_name(value) {
            return Err(OrbitError::InvalidInput(format!(
                "execution.env.pass contains invalid variable name '{value}'"
            )));
        }
        normalized.insert(value.to_string());
    }
    Ok(normalized.into_iter().collect())
}

fn is_valid_env_var_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn required_env_vars_for_provider(provider: &str) -> &'static [&'static str] {
    match provider {
        "codex" => &["HOME", "PATH"],
        "claude" => &["HOME", "PATH"],
        _ => &[],
    }
}

fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return Some(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.trim().is_empty()
    {
        return Some(PathBuf::from(profile));
    }
    None
}
