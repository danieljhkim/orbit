use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use orbit_types::OrbitError;

use crate::paths;

use super::persistence::PersistenceConfig;
use super::raw::{
    RawCodexExecutionConfig, RawExecutionEnvConfig, RawRuntimeConfig, RawTaskSection,
    RawUserSection,
};

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT: bool = false;
const DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL: bool = false;
const DEFAULT_USER_NAME: &str = "human";

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) codex_execution: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval: TaskApprovalConfig,
    pub(crate) user_name: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::default_for_data_root(&paths::current_dir_orbit_root())
    }
}

impl RuntimeConfig {
    pub(crate) fn default_for_data_root(data_root: &Path) -> Self {
        Self {
            execution_env: ExecutionEnvPolicy::default(),
            codex_execution: CodexExecutionPolicy::default(),
            persistence: PersistenceConfig::default_for_data_root(data_root),
            task_approval: TaskApprovalConfig::default(),
            user_name: DEFAULT_USER_NAME.to_string(),
        }
    }

    pub(crate) fn load_from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        let config_path = data_root.join("config.toml");
        if !config_path.exists() {
            return Ok(Self::default_for_data_root(data_root));
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
            codex_execution: CodexExecutionPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.codex),
            )?,
            persistence: PersistenceConfig::from_raw(data_root, &parsed)?,
            task_approval: TaskApprovalConfig::from_raw(parsed.task.as_ref())?,
            user_name: parse_user_name(parsed.user.as_ref())?,
        })
    }
}

fn parse_user_name(raw: Option<&RawUserSection>) -> Result<String, OrbitError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_USER_NAME.to_string());
    };
    let Some(name) = raw.name.as_deref() else {
        return Ok(DEFAULT_USER_NAME.to_string());
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(OrbitError::InvalidInput(
            "user.name must not be empty when configured".to_string(),
        ));
    }
    Ok(name.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexExecutionPolicy {
    sandbox: CodexSandboxMode,
    approval_policy: Option<CodexApprovalPolicy>,
}

impl Default for CodexExecutionPolicy {
    fn default() -> Self {
        Self {
            sandbox: CodexSandboxMode::WorkspaceWrite,
            approval_policy: None,
        }
    }
}

impl CodexExecutionPolicy {
    fn from_raw(raw: Option<RawCodexExecutionConfig>) -> Result<Self, OrbitError> {
        let Some(raw) = raw else {
            return Ok(Self::default());
        };

        let sandbox = match raw.sandbox.as_deref() {
            Some(value) => CodexSandboxMode::parse(value)?,
            None => CodexSandboxMode::WorkspaceWrite,
        };
        let approval_policy = raw
            .approval_policy
            .as_deref()
            .map(CodexApprovalPolicy::parse)
            .transpose()?;

        Ok(Self {
            sandbox,
            approval_policy,
        })
    }

    pub(crate) fn sandbox(&self) -> &str {
        self.sandbox.as_str()
    }

    pub(crate) fn approval_policy(&self) -> Option<&str> {
        self.approval_policy.map(CodexApprovalPolicy::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexSandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl CodexSandboxMode {
    fn parse(value: &str) -> Result<Self, OrbitError> {
        match value.trim() {
            "read-only" => Ok(Self::ReadOnly),
            "workspace-write" => Ok(Self::WorkspaceWrite),
            "danger-full-access" => Ok(Self::DangerFullAccess),
            other => Err(OrbitError::InvalidInput(format!(
                "execution.codex.sandbox has invalid value '{other}'; expected one of: read-only, workspace-write, danger-full-access"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
}

impl CodexApprovalPolicy {
    fn parse(value: &str) -> Result<Self, OrbitError> {
        match value.trim() {
            "untrusted" => Ok(Self::Untrusted),
            "on-request" => Ok(Self::OnRequest),
            "never" => Ok(Self::Never),
            other => Err(OrbitError::InvalidInput(format!(
                "execution.codex.approval_policy has invalid value '{other}'; expected one of: untrusted, on-request, never"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
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

    pub(crate) fn hydrated_allowlist_env_with_extras(
        &self,
        extras: &[String],
    ) -> Vec<(String, String)> {
        let mut names: std::collections::BTreeSet<&str> =
            self.pass.iter().map(String::as_str).collect();
        names.extend(extras.iter().map(String::as_str));
        names
            .iter()
            .filter_map(|name| {
                std::env::var(*name)
                    .ok()
                    .map(|value| (name.to_string(), value))
            })
            .collect()
    }

    pub(crate) fn missing_required(&self, required_env_vars: &[&str]) -> Vec<String> {
        required_env_vars
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

fn default_pass_list() -> Vec<String> {
    // Cross-platform POSIX base: required by virtually all CLI tools.
    let mut vars: Vec<&str> = vec!["HOME", "PATH", "CODEX_HOME", "TMPDIR", "USER"];

    // macOS: SCDynamicStore / CoreFoundation requires this encoding var.
    // Without it, agent CLIs that link system-configuration panic with
    // "Attempted to create a NULL object".
    #[cfg(target_os = "macos")]
    vars.push("__CF_USER_TEXT_ENCODING");

    vars.iter().map(ToString::to_string).collect()
}

pub(crate) fn normalize_pass_list(pass: Vec<String>) -> Result<Vec<String>, OrbitError> {
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
