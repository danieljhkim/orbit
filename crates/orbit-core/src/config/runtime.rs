use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use orbit_common::types::{
    Crew, CrewRoleAssignment, OrbitError, activity_job::Backend, all_agent_families, resolve_crew,
};
use orbit_common::utility::redaction::redact_home_dir;
use orbit_engine::PrConfig;

use crate::paths;

use super::persistence::PersistenceConfig;
use super::raw::{
    RawAgentRoleConfig, RawCodexExecutionConfig, RawCrewEntry, RawDuelSection,
    RawExecutionEnvConfig, RawPrSection, RawRuntimeConfig, RawRuntimeSection, RawTaskSection,
    RawWorkflowConfig,
};

const DEFAULT_ENV_INHERIT: bool = false;
const DEFAULT_TASK_APPROVAL_REQUIRED_FOR_AGENT: bool = false;
const DEFAULT_TASK_APPROVAL_DELEGATE_APPROVAL: bool = false;
// Keep the runtime fallback aligned with the seeded default config so repos
// without an explicit Orbit config still record scoreboard metrics.
const DEFAULT_SCORING_ENABLED: bool = true;
const DEFAULT_GRAPH_EDITING: bool = false;
const DEFAULT_WORKFLOW_BASE_BRANCH: &str = "main";
const DEFAULT_WORKFLOW_CREW: &str = "codex";

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) codex_execution: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) task_approval: TaskApprovalConfig,
    pub(crate) pr: PrConfig,
    pub(crate) scoring_enabled: bool,
    pub(crate) graph_editing: bool,
    /// Persisted default for the v2 `agent_loop` execution backend (§3.1).
    /// `None` means "not configured"; the resolver falls through to the hard-
    /// coded `cli` default.
    pub(crate) v2_backend: Option<String>,
    /// Default base branch for ship/duel-plan workflows. Sourced
    /// from `[workflow] base_branch` in `config.toml`; defaults to `"main"`
    /// when no key is set.
    pub(crate) workflow_base_branch: String,
    /// Named planner/implementer/reviewer lineups from `[crews.<name>]`.
    pub(crate) crews: BTreeMap<String, Crew>,
    pub(crate) default_crew: Option<String>,
    pub(crate) duel: DuelConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DuelConfig {
    pub(crate) candidates: Vec<String>,
    pub(crate) models: BTreeMap<String, String>,
}

impl Default for DuelConfig {
    fn default() -> Self {
        Self {
            candidates: all_agent_families()
                .iter()
                .map(|family| (*family).to_string())
                .collect(),
            models: BTreeMap::new(),
        }
    }
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
            pr: PrConfig::default(),
            scoring_enabled: DEFAULT_SCORING_ENABLED,
            graph_editing: DEFAULT_GRAPH_EDITING,
            v2_backend: None,
            workflow_base_branch: DEFAULT_WORKFLOW_BASE_BRANCH.to_string(),
            crews: default_crews(),
            default_crew: Some(DEFAULT_WORKFLOW_CREW.to_string()),
            duel: DuelConfig::default(),
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

        validate_task_artifact_store_from_raw(parsed.task.as_ref())?;
        let v2_backend = runtime_backend_from_raw(parsed.runtime.as_ref())?;

        reject_stale_agent_role_tables(parsed.agent.as_ref())?;

        let workflow_base_branch = workflow_base_branch_from_raw(parsed.workflow.as_ref())?;
        let crews = crews_from_raw(parsed.crews.as_ref())?;
        let default_crew = workflow_default_crew_from_raw(parsed.workflow.as_ref(), &crews)?;
        let duel = duel_from_raw(parsed.duel.as_ref())?;
        let pr = pr_config_from_raw(parsed.pr.as_ref());

        if parsed
            .knowledge
            .as_ref()
            .and_then(|section| section.task_id_pattern.as_ref())
            .is_some()
        {
            warn_deprecated_task_id_pattern(&config_path);
        }

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.env),
            )?,
            codex_execution: CodexExecutionPolicy::from_raw(
                parsed.execution.clone().and_then(|v| v.codex),
            )?,
            persistence,
            task_approval: TaskApprovalConfig::from_raw(parsed.task.as_ref())?,
            pr,
            scoring_enabled,
            graph_editing,
            v2_backend,
            workflow_base_branch,
            crews,
            default_crew,
            duel,
        })
    }

    /// Configured default backend for v2 `agent_loop` activities (§3.1 step 3).
    pub(crate) fn v2_backend(&self) -> Option<&str> {
        self.v2_backend.as_deref()
    }

    pub(crate) fn workflow_base_branch(&self) -> &str {
        &self.workflow_base_branch
    }

    pub(crate) fn pr_config(&self) -> &PrConfig {
        &self.pr
    }

    pub(crate) fn duel_config(&self) -> &DuelConfig {
        &self.duel
    }
}

pub(crate) fn default_crews() -> BTreeMap<String, Crew> {
    let mut crews = BTreeMap::new();
    crews.insert(
        "claude".to_string(),
        Crew {
            name: "claude".to_string(),
            planner: crew_role("claude-opus-4-7", "claude", "cli"),
            implementer: crew_role("claude-opus-4-7", "claude", "cli"),
            reviewer: crew_role("claude-opus-4-7", "claude", "cli"),
        },
    );
    crews.insert(
        "codex".to_string(),
        Crew {
            name: "codex".to_string(),
            planner: crew_role("gpt-5.5", "codex", "cli"),
            implementer: crew_role("gpt-5.5", "codex", "cli"),
            reviewer: crew_role("gpt-5.5", "codex", "cli"),
        },
    );
    crews.insert(
        "gemini".to_string(),
        Crew {
            name: "gemini".to_string(),
            planner: crew_role("pro", "gemini", "cli"),
            implementer: crew_role("pro", "gemini", "cli"),
            reviewer: crew_role("pro", "gemini", "cli"),
        },
    );
    crews.insert(
        "grok".to_string(),
        Crew {
            name: "grok".to_string(),
            planner: crew_role("grok-build", "grok", "cli"),
            implementer: crew_role("grok-build", "grok", "cli"),
            reviewer: crew_role("grok-build", "grok", "cli"),
        },
    );
    crews
}

fn crew_role(model: &str, provider: &str, backend: &str) -> CrewRoleAssignment {
    CrewRoleAssignment {
        model: model.to_string(),
        provider: provider.to_string(),
        backend: backend.to_string(),
    }
}

fn pr_config_from_raw(raw: Option<&RawPrSection>) -> PrConfig {
    PrConfig {
        task_url_template: raw.and_then(|section| section.task_url_template.clone()),
    }
}

fn duel_from_raw(raw: Option<&RawDuelSection>) -> Result<DuelConfig, OrbitError> {
    let Some(raw) = raw else {
        return Ok(DuelConfig::default());
    };

    let candidates = duel_candidates_from_raw(raw.candidates.as_deref())?;
    let models = duel_models_from_raw(raw.models.as_ref(), &candidates)?;
    Ok(DuelConfig { candidates, models })
}

fn duel_candidates_from_raw(raw: Option<&[String]>) -> Result<Vec<String>, OrbitError> {
    let Some(raw_candidates) = raw else {
        return Ok(DuelConfig::default().candidates);
    };
    if raw_candidates.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "[duel] candidates must contain at least 3 entries; valid candidates: {}",
            valid_duel_candidates()
        )));
    }

    let valid: BTreeSet<&str> = all_agent_families().into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for candidate in raw_candidates {
        let normalized = candidate.trim().to_ascii_lowercase();
        if !seen.insert(normalized.clone()) {
            return Err(OrbitError::InvalidInput(format!(
                "[duel] candidates contains duplicate '{normalized}' after normalization; valid candidates: {}",
                valid_duel_candidates()
            )));
        }
        if !valid.contains(normalized.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "[duel] candidates contains unknown entry '{normalized}'; valid candidates: {}",
                valid_duel_candidates()
            )));
        }
        candidates.push(normalized);
    }

    if candidates.len() < 3 {
        return Err(OrbitError::InvalidInput(format!(
            "[duel] candidates must contain at least 3 distinct entries after normalization (got {}: {}); valid candidates: {}",
            candidates.len(),
            candidates.join(", "),
            valid_duel_candidates()
        )));
    }

    Ok(candidates)
}

fn duel_models_from_raw(
    raw: Option<&BTreeMap<String, String>>,
    candidates: &[String],
) -> Result<BTreeMap<String, String>, OrbitError> {
    let Some(raw_models) = raw else {
        return Ok(BTreeMap::new());
    };
    let candidate_set: BTreeSet<&str> = candidates.iter().map(String::as_str).collect();
    let candidate_list = candidates.join(", ");
    let mut models = BTreeMap::new();
    for (family, model) in raw_models {
        let normalized_family = family.trim().to_ascii_lowercase();
        if !candidate_set.contains(normalized_family.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "[duel.models] contains key '{normalized_family}' that is not in resolved [duel].candidates ({candidate_list}); valid candidates: {}",
                valid_duel_candidates()
            )));
        }
        let trimmed_model = model.trim();
        if trimmed_model.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "[duel.models].{normalized_family} must not be empty (found '{model}'); configured candidates: {candidate_list}"
            )));
        }
        models.insert(normalized_family, trimmed_model.to_string());
    }
    Ok(models)
}

fn valid_duel_candidates() -> String {
    all_agent_families().join(", ")
}

fn reject_stale_agent_role_tables(
    raw: Option<&BTreeMap<String, RawAgentRoleConfig>>,
) -> Result<(), OrbitError> {
    if raw.is_some() {
        return Err(OrbitError::InvalidInput(
            "config schema changed in ORB-00058; remove [agent.<role>] tables and migrate to [crews.<name>] with [workflow].default_crew".to_string(),
        ));
    }
    Ok(())
}

fn crews_from_raw(
    raw: Option<&BTreeMap<String, RawCrewEntry>>,
) -> Result<BTreeMap<String, Crew>, OrbitError> {
    let Some(raw_crews) = raw else {
        return Ok(default_crews());
    };
    let mut crews = BTreeMap::new();
    for (name, entry) in raw_crews {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(OrbitError::InvalidInput(
                "[crews] names must not be empty".to_string(),
            ));
        }
        let crew = Crew {
            name: trimmed.to_string(),
            planner: required_role_assignment(trimmed, "planner", entry.planner.as_ref())?,
            implementer: required_role_assignment(
                trimmed,
                "implementer",
                entry.implementer.as_ref(),
            )?,
            reviewer: required_role_assignment(trimmed, "reviewer", entry.reviewer.as_ref())?,
        };
        crews.insert(trimmed.to_string(), crew);
    }
    if crews.is_empty() {
        return Err(OrbitError::InvalidInput(
            "[crews] must define at least one crew".to_string(),
        ));
    }
    Ok(crews)
}

fn required_role_assignment(
    crew: &str,
    role: &str,
    raw: Option<&RawAgentRoleConfig>,
) -> Result<CrewRoleAssignment, OrbitError> {
    let raw = raw.ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "[crews.{crew}] must define {role} = {{ model, provider, backend }}"
        ))
    })?;
    Ok(CrewRoleAssignment {
        model: required_role_field(crew, role, "model", raw.model.as_deref())?,
        provider: required_role_field(crew, role, "provider", raw.provider.as_deref())?,
        backend: required_role_field(crew, role, "backend", raw.backend.as_deref())?,
    })
}

fn required_role_field(
    crew: &str,
    role: &str,
    field: &str,
    value: Option<&str>,
) -> Result<String, OrbitError> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    value.map(ToOwned::to_owned).ok_or_else(|| {
        OrbitError::InvalidInput(format!("[crews.{crew}].{role}.{field} must not be empty"))
    })
}

pub(super) fn workflow_default_crew_from_raw(
    raw: Option<&RawWorkflowConfig>,
    crews: &BTreeMap<String, Crew>,
) -> Result<Option<String>, OrbitError> {
    let value = raw.and_then(|workflow| workflow.default_crew.as_deref());
    let Some(value) = value else {
        // No explicit [workflow].default_crew. Fall back to the seeded default
        // when its crew is still present; otherwise demand the user pick one
        // explicitly so downstream `start`/`show` calls don't surprise them
        // with a generic "no crew selected" error.
        if crews.contains_key(DEFAULT_WORKFLOW_CREW) {
            return Ok(Some(DEFAULT_WORKFLOW_CREW.to_string()));
        }
        if crews.is_empty() {
            return Ok(None);
        }
        let mut names: Vec<&str> = crews.keys().map(String::as_str).collect();
        names.sort();
        return Err(OrbitError::InvalidInput(format!(
            "[workflow].default_crew must be set when defining [crews.*]; choose one of: {}",
            names.join(", ")
        )));
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "workflow.default_crew must not be empty".to_string(),
        ));
    }
    resolve_crew(trimmed, crews)?;
    Ok(Some(trimmed.to_string()))
}

fn runtime_backend_from_raw(raw: Option<&RawRuntimeSection>) -> Result<Option<String>, OrbitError> {
    let Some(value) = raw.and_then(|section| section.backend.as_deref()) else {
        return Ok(None);
    };
    let trimmed = value.trim();
    let Some(backend) = Backend::parse(trimmed) else {
        return Err(OrbitError::InvalidInput(format!(
            "[runtime] backend has invalid value '{trimmed}'; expected one of: http, cli, auto"
        )));
    };
    Ok(Some(backend.as_str().to_string()))
}

fn validate_task_artifact_store_from_raw(raw: Option<&RawTaskSection>) -> Result<(), OrbitError> {
    let Some(value) = raw.and_then(|section| section.artifact_store.as_deref()) else {
        return Ok(());
    };
    let trimmed = value.trim();
    Err(OrbitError::InvalidInput(format!(
        "[task] artifact_store is no longer supported; remove the key because v2 task artifacts are always enabled (found '{trimmed}')"
    )))
}

fn workflow_base_branch_from_raw(raw: Option<&RawWorkflowConfig>) -> Result<String, OrbitError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_WORKFLOW_BASE_BRANCH.to_string());
    };
    let Some(value) = raw.base_branch.as_deref() else {
        return Ok(DEFAULT_WORKFLOW_BASE_BRANCH.to_string());
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "workflow.base_branch must not be empty".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn warn_deprecated_task_id_pattern(config_path: &Path) {
    let path = redact_home_dir(&config_path.display().to_string());
    tracing::warn!(
        config = %path,
        "knowledge.task_id_pattern is deprecated and ignored",
    );
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
                // Env inheritance is fixed at DEFAULT_ENV_INHERIT and cannot be
                // overridden by config — an untrusted workspace config.toml must
                // not be able to widen passthrough to the full host environment
                // (secrets/tokens). Only the `pass` allowlist is configurable.
                // See ORB-00365.
                let pass = normalize_pass_list(raw.pass.unwrap_or_else(default_pass_list))?;
                Ok(Self {
                    inherit: DEFAULT_ENV_INHERIT,
                    pass,
                })
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
