use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use orbit_common::model_defaults::{
    CLAUDE_DEFAULT_WEAK, CODEX_DEFAULT_MODEL, GEMINI_CREW_MODEL, GROK_DEFAULT_MODEL,
};
use orbit_common::types::{Crew, CrewRoleAssignment, OrbitError, all_agent_families};
use orbit_common::utility::redaction::redact_home_dir;
use orbit_engine::PrConfig;

use crate::paths;

use super::persistence::PersistenceConfig;
use super::raw::{RawAgentRoleConfig, RawCrewEntry, RawRuntimeConfig, RawTaskSection};
use super::registry::ConfigSnapshot;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) snapshot: ConfigSnapshot,
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
    /// Opt-in for unattended ship dispatch (`[workflow] auto_ship` in
    /// `config.toml`; defaults to `false`).
    pub(crate) workflow_auto_ship: bool,
    /// Whether this workspace is a routine source (`[routines] role =
    /// "source"` in `config.toml`; defaults to `false`). Consulted by
    /// `orbit sweep` before loading `.orbit/routines/*.yaml`.
    pub(crate) routines_source: bool,
    /// Named provider-model assignments from `[crews.<name>]`.
    pub(crate) crews: BTreeMap<String, Crew>,
    pub(crate) default_crew: Option<String>,
    pub(crate) duel: DuelConfig,
    /// Optional floor for the local task-id allocator (`[tasks] id_start`).
    /// Applied forward-only on runtime build so machines can hold disjoint id
    /// ranges. `None` leaves the allocator untouched.
    pub(crate) tasks_id_start: Option<u32>,
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
        let snapshot = ConfigSnapshot::default();
        Self {
            execution_env: ExecutionEnvPolicy::from_snapshot(&snapshot),
            codex_execution: CodexExecutionPolicy::from_snapshot(&snapshot),
            persistence: PersistenceConfig::default_for_data_root(data_root),
            task_approval: TaskApprovalConfig::from_snapshot(&snapshot),
            pr: PrConfig {
                task_url_template: snapshot.pr_task_url_template.clone(),
            },
            scoring_enabled: snapshot.scoring_enabled,
            graph_editing: snapshot.graph_editing,
            v2_backend: snapshot.runtime_backend.clone(),
            workflow_base_branch: snapshot.workflow_base_branch.clone(),
            workflow_auto_ship: snapshot.workflow_auto_ship,
            routines_source: snapshot.routines_role.as_deref() == Some("source"),
            crews: default_crews(),
            default_crew: snapshot.workflow_default_crew.clone(),
            duel: DuelConfig {
                candidates: snapshot.duel_candidates.clone(),
                models: snapshot.duel_models.clone(),
            },
            tasks_id_start: snapshot.tasks_id_start,
            snapshot,
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
        Self::from_raw_str(&raw, &config_path, persistence)
    }

    /// Parse and validate a raw `config.toml` document string into a fully
    /// resolved [`RuntimeConfig`], running it through the exact same
    /// validation pipeline as [`Self::load_layered`].
    ///
    /// `config_path` is used only to build human-readable error messages
    /// (it need not exist on disk — this is also the entry point used by
    /// `ConfigStore::validate` to check an in-memory edit before it is
    /// written to disk). `persistence` is supplied by the caller because
    /// persistence paths are derived from the two data roots, not from the
    /// config document itself.
    pub(crate) fn from_raw_str(
        raw: &str,
        config_path: &Path,
        persistence: PersistenceConfig,
    ) -> Result<Self, OrbitError> {
        let parsed = toml::from_str::<RawRuntimeConfig>(raw).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid runtime config '{}': {err}",
                redact_home_dir(&config_path.display().to_string())
            ))
        })?;
        let document = toml::from_str::<toml::Value>(raw).map_err(|err| {
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

        validate_task_artifact_store_from_raw(parsed.task.as_ref())?;
        reject_stale_agent_role_tables(parsed.agent.as_ref())?;
        let crews = crews_from_raw(parsed.crews.as_ref())?;
        let snapshot = ConfigSnapshot::admit(&document, config_path, &crews)?;

        if parsed
            .knowledge
            .as_ref()
            .and_then(|section| section.task_id_pattern.as_ref())
            .is_some()
        {
            warn_deprecated_task_id_pattern(config_path);
        }

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_snapshot(&snapshot),
            codex_execution: CodexExecutionPolicy::from_snapshot(&snapshot),
            persistence,
            task_approval: TaskApprovalConfig::from_snapshot(&snapshot),
            pr: PrConfig {
                task_url_template: snapshot.pr_task_url_template.clone(),
            },
            scoring_enabled: snapshot.scoring_enabled,
            graph_editing: snapshot.graph_editing,
            v2_backend: snapshot.runtime_backend.clone(),
            workflow_base_branch: snapshot.workflow_base_branch.clone(),
            workflow_auto_ship: snapshot.workflow_auto_ship,
            routines_source: snapshot.routines_role.as_deref() == Some("source"),
            crews,
            default_crew: snapshot.workflow_default_crew.clone(),
            duel: DuelConfig {
                candidates: snapshot.duel_candidates.clone(),
                models: snapshot.duel_models.clone(),
            },
            tasks_id_start: snapshot.tasks_id_start,
            snapshot,
        })
    }

    /// Configured `[tasks] id_start` floor, if any.
    pub(crate) fn tasks_id_start(&self) -> Option<u32> {
        self.tasks_id_start
    }

    /// Configured default backend for v2 `agent_loop` activities (§3.1 step 3).
    pub(crate) fn v2_backend(&self) -> Option<&str> {
        self.v2_backend.as_deref()
    }

    pub(crate) fn workflow_base_branch(&self) -> &str {
        &self.workflow_base_branch
    }

    pub(crate) fn workflow_auto_ship(&self) -> bool {
        self.workflow_auto_ship
    }

    pub(crate) fn routines_source(&self) -> bool {
        self.routines_source
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
            assignment: crew_role(CLAUDE_DEFAULT_WEAK, "claude", "cli"),
            description: None,
            tags: Vec::new(),
        },
    );
    crews.insert(
        "codex".to_string(),
        Crew {
            name: "codex".to_string(),
            assignment: crew_role(CODEX_DEFAULT_MODEL, "codex", "cli"),
            description: None,
            tags: Vec::new(),
        },
    );
    crews.insert(
        "gemini".to_string(),
        Crew {
            name: "gemini".to_string(),
            assignment: crew_role(GEMINI_CREW_MODEL, "gemini", "cli"),
            description: None,
            tags: Vec::new(),
        },
    );
    crews.insert(
        "grok".to_string(),
        Crew {
            name: "grok".to_string(),
            assignment: crew_role(GROK_DEFAULT_MODEL, "grok", "cli"),
            description: None,
            tags: Vec::new(),
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
            assignment: crew_assignment_from_raw(trimmed, entry)?,
            description: normalized_crew_description(entry.description.as_deref()),
            tags: normalized_crew_tags(&entry.tags),
        };
        if crews.insert(trimmed.to_string(), crew).is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "[crews] contains duplicate name '{trimmed}' after whitespace normalization"
            )));
        }
    }
    if crews.is_empty() {
        return Err(OrbitError::InvalidInput(
            "[crews] must define at least one crew".to_string(),
        ));
    }
    Ok(crews)
}

fn normalized_crew_description(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalized_crew_tags(raw: &[String]) -> Vec<String> {
    let mut tags = raw
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

fn crew_assignment_from_raw(
    crew: &str,
    raw: &RawCrewEntry,
) -> Result<CrewRoleAssignment, OrbitError> {
    let has_flat = raw.model.is_some() || raw.provider.is_some() || raw.backend.is_some();
    let has_legacy = raw.planner.is_some() || raw.implementer.is_some() || raw.reviewer.is_some();
    if has_flat && has_legacy {
        return Err(OrbitError::InvalidInput(format!(
            "[crews.{crew}] mixes the flat {{ model, provider, backend }} shape with legacy planner/implementer/reviewer assignments"
        )));
    }
    if has_flat {
        return Ok(CrewRoleAssignment {
            model: required_crew_field(crew, "model", raw.model.as_deref())?,
            provider: required_crew_field(crew, "provider", raw.provider.as_deref())?,
            backend: required_crew_field(crew, "backend", raw.backend.as_deref())?,
        });
    }

    let implementer = required_legacy_assignment(crew, "implementer", raw.implementer.as_ref())?;
    let planner = required_legacy_assignment(crew, "planner", raw.planner.as_ref())?;
    let reviewer = required_legacy_assignment(crew, "reviewer", raw.reviewer.as_ref())?;
    if planner != implementer || reviewer != implementer {
        tracing::warn!(
            target: "orbit.config.crew",
            crew,
            "legacy three-role crew assignments diverge; using implementer for every role — rewrite [crews.<name>] with flat model/provider/backend fields",
        );
    }
    Ok(implementer)
}

fn required_legacy_assignment(
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
        model: required_legacy_field(crew, role, "model", raw.model.as_deref())?,
        provider: required_legacy_field(crew, role, "provider", raw.provider.as_deref())?,
        backend: required_legacy_field(crew, role, "backend", raw.backend.as_deref())?,
    })
}

fn required_legacy_field(
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

fn required_crew_field(crew: &str, field: &str, value: Option<&str>) -> Result<String, OrbitError> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    value.map(ToOwned::to_owned).ok_or_else(|| {
        OrbitError::InvalidInput(format!("[crews.{crew}].{field} must not be empty"))
    })
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

fn warn_deprecated_task_id_pattern(config_path: &Path) {
    let path = redact_home_dir(&config_path.display().to_string());
    tracing::warn!(
        config = %path,
        "knowledge.task_id_pattern is deprecated and ignored",
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexExecutionPolicy {
    sandbox: String,
    approval_policy: Option<String>,
}

impl Default for CodexExecutionPolicy {
    fn default() -> Self {
        Self {
            sandbox: "workspace-write".to_string(),
            approval_policy: None,
        }
    }
}

impl CodexExecutionPolicy {
    fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        Self {
            sandbox: snapshot.codex_sandbox.clone(),
            approval_policy: snapshot.codex_approval_policy.clone(),
        }
    }

    pub(crate) fn sandbox(&self) -> &str {
        &self.sandbox
    }

    pub(crate) fn approval_policy(&self) -> Option<&str> {
        self.approval_policy.as_deref()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskApprovalConfig {
    pub(crate) required_for_agent: bool,
    pub(crate) delegate_approval: bool,
}

impl TaskApprovalConfig {
    fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        Self {
            required_for_agent: snapshot.task_approval_required_for_agent,
            delegate_approval: snapshot.task_delegate_approval,
        }
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
            inherit: false,
            pass: default_pass_list(),
        }
    }
}

impl ExecutionEnvPolicy {
    fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        Self {
            inherit: snapshot.execution_env_inherit,
            pass: snapshot.execution_env_pass.clone(),
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
    ConfigSnapshot::default().execution_env_pass
}

fn cli_command_baseline_pass_list() -> Vec<String> {
    let mut vars = default_pass_list();
    vars.push("LANG".to_string());
    vars.push("TZ".to_string());
    vars.sort();
    vars.dedup();
    vars
}
