use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use orbit_types::redaction::redact_home_dir;
use orbit_types::{AgentModelPair, OrbitError, agent_family_from_cli, resolve_agent_model_pair};

use crate::paths;

use super::persistence::PersistenceConfig;
use super::raw::{
    RawAgentAssignment, RawAgentModelEntry, RawAgentModelsConfig, RawCodexExecutionConfig,
    RawExecutionEnvConfig, RawRuntimeConfig, RawShipWorkflowConfig, RawTaskSection,
    RawWorkflowConfig,
};

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT: bool = false;
const DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL: bool = false;
const DEFAULT_SCORING_ENABLED: bool = false;
const DEFAULT_GRAPH_EDITING: bool = false;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentModelEntry {
    pub(crate) strong: String,
    pub(crate) weak: String,
}

impl AgentModelEntry {
    fn new(strong: impl Into<String>, weak: impl Into<String>) -> Self {
        Self {
            strong: strong.into(),
            weak: weak.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentAssignment {
    pub(crate) agent: String,
    pub(crate) model: String,
}

impl AgentAssignment {
    fn new(agent: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            model: model.into(),
        }
    }

    fn from_raw(raw: RawAgentAssignment) -> Result<Self, OrbitError> {
        let agent = raw.agent.trim();
        let model = raw.model.trim();
        if agent.is_empty() || model.is_empty() {
            return Err(OrbitError::InvalidInput(
                "workflow.ship role assignments require non-empty agent and model".to_string(),
            ));
        }
        Ok(Self::new(agent, model))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShipWorkflowConfig {
    pub(crate) plan: AgentAssignment,
    pub(crate) implement: AgentAssignment,
    pub(crate) review: AgentAssignment,
    pub(crate) finalize: AgentAssignment,
}

impl Default for ShipWorkflowConfig {
    fn default() -> Self {
        Self {
            plan: AgentAssignment::new("claude", "opus-4.6"),
            implement: AgentAssignment::new("codex", "gpt-5.4"),
            review: AgentAssignment::new("claude", "sonnet-4.6"),
            finalize: AgentAssignment::new("gemini", "gemini-3.1-pro-preview"),
        }
    }
}

impl ShipWorkflowConfig {
    fn from_raw(raw: Option<RawShipWorkflowConfig>) -> Result<Self, OrbitError> {
        let defaults = Self::default();
        let Some(raw) = raw else {
            return Ok(defaults);
        };

        Ok(Self {
            plan: raw
                .plan
                .map(AgentAssignment::from_raw)
                .transpose()?
                .unwrap_or(defaults.plan),
            implement: raw
                .implement
                .map(AgentAssignment::from_raw)
                .transpose()?
                .unwrap_or(defaults.implement),
            review: raw
                .review
                .map(AgentAssignment::from_raw)
                .transpose()?
                .unwrap_or(defaults.review),
            finalize: raw
                .finalize
                .map(AgentAssignment::from_raw)
                .transpose()?
                .unwrap_or(defaults.finalize),
        })
    }

    pub(crate) fn role(&self, role: &str) -> Option<&AgentAssignment> {
        match role {
            "plan" => Some(&self.plan),
            "implement" => Some(&self.implement),
            "review" => Some(&self.review),
            "finalize" => Some(&self.finalize),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct WorkflowConfig {
    pub(crate) ship: ShipWorkflowConfig,
}

impl WorkflowConfig {
    fn from_raw(raw: Option<RawWorkflowConfig>) -> Result<Self, OrbitError> {
        Ok(Self {
            ship: ShipWorkflowConfig::from_raw(raw.and_then(|workflow| workflow.ship))?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentModelsConfig {
    pub(crate) claude: AgentModelEntry,
    pub(crate) codex: AgentModelEntry,
    pub(crate) gemini: AgentModelEntry,
}

impl Default for AgentModelsConfig {
    fn default() -> Self {
        Self {
            claude: AgentModelEntry::new("opus-4.6", "sonnet-4.6"),
            codex: AgentModelEntry::new("gpt-5.4", "gpt-5.4-mini"),
            gemini: AgentModelEntry::new("gemini-3.1-pro-preview", "gemini-3-flash-preview"),
        }
    }
}

impl AgentModelsConfig {
    fn from_raw(raw: Option<RawAgentModelsConfig>) -> Result<Self, OrbitError> {
        let defaults = Self::default();
        let Some(raw) = raw else {
            return Ok(defaults);
        };

        Ok(Self {
            claude: merge_agent_model_entry(defaults.claude, raw.claude)?,
            codex: merge_agent_model_entry(defaults.codex, raw.codex)?,
            gemini: merge_agent_model_entry(defaults.gemini, raw.gemini)?,
        })
    }

    pub(crate) fn pair_for(&self, family: &str) -> Option<AgentModelPair> {
        self.entry(family)
            .map(|entry| AgentModelPair::new(entry.strong.clone(), entry.weak.clone()))
    }

    pub(crate) fn canonical_model_name(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Option<String> {
        let requested = model.map(str::trim).filter(|value| !value.is_empty())?;
        let family = agent_family_from_cli(agent_cli);
        let Some(entry) = self.entry(&family) else {
            return Some(requested.to_string());
        };

        if matches_model_alias(&family, requested, &entry.strong, true) {
            return Some(entry.strong.clone());
        }
        if matches_model_alias(&family, requested, &entry.weak, false) {
            return Some(entry.weak.clone());
        }

        Some(requested.to_string())
    }

    fn entry(&self, family: &str) -> Option<&AgentModelEntry> {
        match family {
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            "gemini" => Some(&self.gemini),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) codex_execution: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval: TaskApprovalConfig,
    pub(crate) agent_models: AgentModelsConfig,
    pub(crate) workflow: WorkflowConfig,
    pub(crate) scoring_enabled: bool,
    pub(crate) graph_editing: bool,
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
            agent_models: AgentModelsConfig::default(),
            workflow: WorkflowConfig::default(),
            scoring_enabled: DEFAULT_SCORING_ENABLED,
            graph_editing: DEFAULT_GRAPH_EDITING,
        }
    }

    #[cfg(test)]
    pub(crate) fn agent_model_pair(&self, family: &str) -> Option<AgentModelPair> {
        self.agent_models.pair_for(&agent_family_from_cli(family))
    }

    #[cfg(test)]
    pub(crate) fn canonical_model_name(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Option<String> {
        self.agent_models.canonical_model_name(agent_cli, model)
    }

    #[cfg(test)]
    pub(crate) fn ship_role_assignment(&self, role: &str) -> Option<AgentAssignment> {
        self.workflow.ship.role(role).cloned()
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

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.env),
            )?,
            codex_execution: CodexExecutionPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.codex),
            )?,
            persistence,
            task_approval: TaskApprovalConfig::from_raw(parsed.task.as_ref())?,
            agent_models: AgentModelsConfig::from_raw(parsed.agents)?,
            workflow: WorkflowConfig::from_raw(parsed.workflow)?,
            scoring_enabled,
            graph_editing,
        })
    }
}

fn merge_agent_model_entry(
    default: AgentModelEntry,
    raw: Option<RawAgentModelEntry>,
) -> Result<AgentModelEntry, OrbitError> {
    let Some(raw) = raw else {
        return Ok(default);
    };

    let strong = raw.strong.trim();
    let weak = raw.weak.trim();
    if strong.is_empty() || weak.is_empty() {
        return Err(OrbitError::InvalidInput(
            "agents.<family> entries require non-empty strong and weak model names".to_string(),
        ));
    }

    Ok(AgentModelEntry::new(strong, weak))
}

fn matches_model_alias(family: &str, requested: &str, configured: &str, strong: bool) -> bool {
    if requested.eq_ignore_ascii_case(configured) {
        return true;
    }

    if let Some(default_pair) = resolve_agent_model_pair(family) {
        let fallback = if strong {
            default_pair.orchestrator
        } else {
            default_pair.helper
        };
        if requested.eq_ignore_ascii_case(&fallback) {
            return true;
        }
    }

    match (family, strong) {
        ("claude", true) => {
            requested.eq_ignore_ascii_case("opus")
                || claude_cli_full_model_name(configured)
                    .is_some_and(|value| requested.eq_ignore_ascii_case(&value))
        }
        ("claude", false) => {
            requested.eq_ignore_ascii_case("sonnet")
                || claude_cli_full_model_name(configured)
                    .is_some_and(|value| requested.eq_ignore_ascii_case(&value))
        }
        ("gemini", true) => requested.eq_ignore_ascii_case("gemini-3.1-pro"),
        ("gemini", false) => requested.eq_ignore_ascii_case("gemini-3-flash"),
        _ => false,
    }
}

fn claude_cli_full_model_name(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if let Some(version) = trimmed.strip_prefix("opus-") {
        return Some(format!("claude-opus-{}", version.replace('.', "-")));
    }
    if let Some(version) = trimmed.strip_prefix("sonnet-") {
        return Some(format!("claude-sonnet-{}", version.replace('.', "-")));
    }
    None
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
    use std::fs;

    use tempfile::tempdir;

    use super::RuntimeConfig;

    #[test]
    fn load_layered_parses_agent_models_and_ship_roles() {
        let global = tempdir().expect("global tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        fs::write(
            workspace.path().join("config.toml"),
            r#"
[agents.claude]
strong = "opus-4.7"
weak = "sonnet-4.7"

[workflow.ship]
plan = { agent = "codex", model = "gpt-5.4" }
review = { agent = "gemini", model = "gemini-3.1-pro-preview" }
"#,
        )
        .expect("write config");

        let config = RuntimeConfig::load_layered(global.path(), workspace.path()).expect("config");
        let claude = config.agent_model_pair("claude").expect("claude pair");
        assert_eq!(claude.orchestrator, "opus-4.7");
        assert_eq!(claude.helper, "sonnet-4.7");

        let plan = config.ship_role_assignment("plan").expect("plan role");
        assert_eq!(plan.agent, "codex");
        assert_eq!(plan.model, "gpt-5.4");

        let review = config.ship_role_assignment("review").expect("review role");
        assert_eq!(review.agent, "gemini");
        assert_eq!(review.model, "gemini-3.1-pro-preview");

        let implement = config
            .ship_role_assignment("implement")
            .expect("implement role");
        assert_eq!(implement.agent, "codex");
        assert_eq!(implement.model, "gpt-5.4");
    }

    #[test]
    fn canonical_model_name_maps_shorthand_to_configured_value() {
        let mut config = RuntimeConfig::default_for_data_root(tempdir().unwrap().path());
        config.agent_models.claude.strong = "opus-4.7".to_string();
        config.agent_models.claude.weak = "sonnet-4.7".to_string();

        assert_eq!(
            config.canonical_model_name("claude", Some("opus")),
            Some("opus-4.7".to_string())
        );
        assert_eq!(
            config.canonical_model_name("claude", Some("sonnet-4.7")),
            Some("sonnet-4.7".to_string())
        );
        assert_eq!(
            config.canonical_model_name("claude", Some("claude-opus-4-7")),
            Some("opus-4.7".to_string())
        );
    }
}
