use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_common::utility::redaction::redact_home_dir;

use crate::paths;

use regex::Regex;

use super::persistence::PersistenceConfig;
use super::raw::{
    RawCodexExecutionConfig, RawExecutionEnvConfig, RawRuntimeConfig, RawTaskSection,
};

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT: bool = false;
const DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL: bool = false;
// Keep the runtime fallback aligned with the seeded default config so repos
// without an explicit Orbit config still record scoreboard metrics.
const DEFAULT_SCORING_ENABLED: bool = true;
const DEFAULT_GRAPH_EDITING: bool = false;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) codex_execution: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval: TaskApprovalConfig,
    pub(crate) scoring_enabled: bool,
    pub(crate) graph_editing: bool,
    /// Persisted default for the v2 `agent_loop` execution backend (§3.1).
    /// `None` means "not configured"; the resolver falls through to the hard-
    /// coded `http` default.
    pub(crate) v2_backend: Option<String>,
    /// `knowledge.task_id_pattern` — workspace override for the task-ID
    /// extraction regex (T20260426-0507). Validated at load time; raw source
    /// string only (avoids forcing an `orbit-knowledge` dep on `orbit-core`).
    pub(crate) task_id_pattern: Option<String>,
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
            scoring_enabled: DEFAULT_SCORING_ENABLED,
            graph_editing: DEFAULT_GRAPH_EDITING,
            v2_backend: None,
            task_id_pattern: None,
        }
    }

    /// Load config with workspace-replaces-global semantics for execution/approval/user.
    ///
    /// Persistence paths are always derived from the two roots (not configurable).
    ///
    /// **Workspace config REPLACES global config** — this is intentional and
    /// different from a merge/layer model. When `workspace_root/config.toml`
    /// exists, it is used exclusively; the `global_root/config.toml` is ignored.
    /// Rationale: per-repo agent behaviour (sandbox mode, approval policy,
    /// allowed env vars) must be fully deterministic and cannot be accidentally
    /// influenced by whatever happens to be in the user's global config.
    /// If workspace_root/config.toml exists, it replaces global config entirely.
    /// Otherwise falls back to global_root/config.toml.
    pub(crate) fn load_layered(
        global_root: &Path,
        workspace_root: &Path,
    ) -> Result<Self, OrbitError> {
        let ws_config = workspace_root.join("config.toml");
        let global_config = global_root.join("config.toml");

        let persistence = PersistenceConfig::default_for_roots(global_root, workspace_root);

        // Workspace config replaces global entirely if present
        let config_path = if ws_config.exists() && workspace_root != global_root {
            ws_config
        } else if global_config.exists() {
            global_config
        } else {
            return Ok(Self {
                persistence,
                ..Self::default_for_data_root(global_root)
            });
        };

        let raw = fs::read_to_string(&config_path).map_err(|err| {
            OrbitError::Io(format!(
                "failed to read runtime config '{}': {err}",
                redact_home_dir(&config_path.display().to_string())
            ))
        })?;
        let parsed = toml::from_str::<RawRuntimeConfig>(&raw).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid runtime config '{}': {err}",
                redact_home_dir(&config_path.display().to_string())
            ))
        })?;

        if parsed.watch.is_some() {
            return Err(OrbitError::InvalidInput(
                "watch config is no longer supported; remove the [watch] section from config.toml"
                    .to_string(),
            ));
        }

        let scoring_enabled = parsed
            .scoring
            .as_ref()
            .and_then(|s| s.enabled)
            .unwrap_or(DEFAULT_SCORING_ENABLED);

        let graph_editing = parsed
            .graph
            .as_ref()
            .and_then(|g| g.editing)
            .unwrap_or(DEFAULT_GRAPH_EDITING);

        let v2_backend = parsed
            .runtime
            .as_ref()
            .and_then(|section| section.backend.clone());

        let task_id_pattern = parsed
            .knowledge
            .as_ref()
            .and_then(|section| section.task_id_pattern.clone())
            .map(|raw| {
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    return Err(OrbitError::InvalidInput(format!(
                        "knowledge.task_id_pattern in '{}' must not be empty",
                        redact_home_dir(&config_path.display().to_string())
                    )));
                }
                Regex::new(&trimmed).map_err(|err| {
                    OrbitError::InvalidInput(format!(
                        "knowledge.task_id_pattern in '{}' is not a valid regex: {err}",
                        redact_home_dir(&config_path.display().to_string())
                    ))
                })?;
                Ok::<String, OrbitError>(trimmed)
            })
            .transpose()?;

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.env),
            )?,
            codex_execution: CodexExecutionPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.codex),
            )?,
            persistence,
            task_approval: TaskApprovalConfig::from_raw(parsed.task.as_ref())?,
            scoring_enabled,
            graph_editing,
            v2_backend,
            task_id_pattern,
        })
    }

    /// Configured default backend for v2 `agent_loop` activities (§3.1 step 3).
    pub(crate) fn v2_backend(&self) -> Option<&str> {
        self.v2_backend.as_deref()
    }

    /// Workspace-configured task-ID extraction regex (T20260426-0507). `None`
    /// means callers should use the Orbit default.
    pub(crate) fn task_id_pattern(&self) -> Option<&str> {
        self.task_id_pattern.as_deref()
    }
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

    pub(crate) fn hydrated_cli_command_env_with_extras(
        &self,
        extras: &[String],
    ) -> Vec<(String, String)> {
        let mut env = std::collections::BTreeMap::new();
        for name in cli_command_baseline_pass_list() {
            if let Ok(value) = std::env::var(&name) {
                env.insert(name.to_string(), value);
            }
        }
        for (name, value) in self.hydrated_allowlist_env_with_extras(extras) {
            env.insert(name, value);
        }
        for (name, value) in std::env::vars() {
            if name.starts_with("ORBIT_") {
                env.insert(name, value);
            }
        }
        env.into_iter().collect()
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
    #[allow(unused_mut)]
    let mut vars: Vec<&str> = vec!["HOME", "PATH", "CODEX_HOME", "TMPDIR", "USER"];

    // macOS: SCDynamicStore / CoreFoundation requires this encoding var.
    // Without it, agent CLIs that link system-configuration panic with
    // "Attempted to create a NULL object".
    #[cfg(target_os = "macos")]
    vars.push("__CF_USER_TEXT_ENCODING");

    vars.iter().map(ToString::to_string).collect()
}

fn cli_command_baseline_pass_list() -> Vec<String> {
    let mut vars = default_pass_list();
    vars.push("LANG".to_string());
    vars.push("TZ".to_string());
    vars.sort();
    vars.dedup();
    vars
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_config(dir: &Path, body: &str) {
        std::fs::write(dir.join("config.toml"), body).expect("write config");
    }

    #[test]
    fn task_id_pattern_loads_valid_regex_from_workspace_config() {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            workspace.path(),
            "[knowledge]\ntask_id_pattern = \"[A-Z]+-\\\\d+\"\n",
        );

        let config =
            RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
        assert_eq!(config.task_id_pattern(), Some(r"[A-Z]+-\d+"));
    }

    #[test]
    fn task_id_pattern_rejects_invalid_regex_at_load_time() {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            workspace.path(),
            "[knowledge]\ntask_id_pattern = \"[unclosed\"\n",
        );

        let err = RuntimeConfig::load_layered(global.path(), workspace.path())
            .expect_err("invalid regex must error");
        let msg = err.to_string();
        assert!(
            msg.contains("knowledge.task_id_pattern") && msg.contains("not a valid regex"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn task_id_pattern_rejects_empty_string() {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        write_config(workspace.path(), "[knowledge]\ntask_id_pattern = \"  \"\n");

        let err = RuntimeConfig::load_layered(global.path(), workspace.path())
            .expect_err("empty pattern must error");
        let msg = err.to_string();
        assert!(
            msg.contains("knowledge.task_id_pattern") && msg.contains("must not be empty"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn task_id_pattern_defaults_to_none_when_section_absent() {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        write_config(workspace.path(), "[scoring]\nenabled = true\n");

        let config =
            RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config loads");
        assert_eq!(config.task_id_pattern(), None);
    }
}
