use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::model_defaults::{
    CLAUDE_DEFAULT_STRONG, CLAUDE_DEFAULT_WEAK, CLAUDE_FABLE_MODEL, CODEX_LUNA_MODEL,
    CODEX_SOL_MODEL, CODEX_TERRA_MODEL, GEMINI_CREW_MODEL, GROK_DEFAULT_MODEL,
};
use orbit_common::types::activity_job::{RETIRED_BACKEND_MIGRATION, check_retired_backend_value};
use orbit_common::types::{Crew, CrewAssignment, OrbitError};
use orbit_common::utility::redaction::redact_home_dir;
use orbit_engine::PrConfig;

use crate::paths;

use super::persistence::PersistenceConfig;
use super::raw::{RawCrewEntry, RawRuntimeConfig, RawTaskSection};
use super::registry::ConfigSnapshot;

const WORKSPACE_REPLACE_ONLY_KEYS: &[&str] = &[
    "execution.codex.approval_policy",
    "execution.codex.sandbox",
    "execution.env.pass",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValueSourceKind {
    BuiltIn,
    Environment,
    Global,
    Workspace,
}

impl ConfigValueSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Environment => "environment",
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValueSource {
    kind: ConfigValueSourceKind,
    path: Option<PathBuf>,
}

impl ConfigValueSource {
    pub fn kind(&self) -> ConfigValueSourceKind {
        self.kind
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct EffectiveConfigValue {
    pub key: String,
    pub value: serde_json::Value,
    pub source: ConfigValueSource,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    snapshot: ConfigSnapshot,
    values: Vec<EffectiveConfigValue>,
}

impl EffectiveConfig {
    pub fn value_for(&self, key: &str) -> Option<serde_json::Value> {
        self.snapshot.value_for(key)
    }

    pub fn values(&self) -> &[EffectiveConfigValue] {
        &self.values
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) snapshot: ConfigSnapshot,
    pub(crate) execution_env: ExecutionEnvPolicy,
    pub(crate) codex_execution: CodexExecutionPolicy,
    pub(crate) persistence: PersistenceConfig,
    pub(crate) pr: PrConfig,
    pub(crate) scoring_enabled: bool,
    /// `None` means "not configured"; the resolver falls through to the hard-
    /// coded `cli` default.
    /// Default base branch for ship workflows. Sourced
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
    pub(crate) system_crew: String,
    /// Optional floor for the local task-id allocator (`[tasks] id_start`).
    /// Applied forward-only on runtime build so machines can hold disjoint id
    /// ranges. `None` leaves the allocator untouched.
    pub(crate) tasks_id_start: Option<u32>,
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
            pr: PrConfig {
                task_url_template: snapshot.pr_task_url_template.clone(),
            },
            scoring_enabled: snapshot.scoring_enabled,
            workflow_base_branch: snapshot.workflow_base_branch.clone(),
            workflow_auto_ship: snapshot.workflow_auto_ship,
            routines_source: snapshot.routines_role.as_deref() == Some("source"),
            crews: default_crews(),
            default_crew: snapshot.workflow_default_crew.clone(),
            system_crew: snapshot.workflow_system_crew.clone(),
            tasks_id_start: snapshot.tasks_id_start,
            snapshot,
        }
    }

    /// Load config with per-key workspace-over-global layering.
    ///
    /// Persistence paths are always derived from the two roots (not configurable).
    ///
    /// Ordinary keys inherit from global when absent from the workspace file.
    /// Sandbox mode, approval policy, and the environment allowlist are the
    /// exception: whenever a distinct workspace file exists, omissions for
    /// those keys resolve to built-in defaults rather than global values.
    pub(crate) fn load_layered(
        global_root: &Path,
        workspace_root: &Path,
    ) -> Result<Self, OrbitError> {
        load_layered_runtime(global_root, workspace_root).map(|loaded| loaded.runtime)
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
        reject_stale_agent_tables(parsed.agent.as_ref())?;
        reject_retired_backend_overrides(
            &document,
            std::env::var(RETIRED_BACKEND_ENV).ok().as_deref(),
        )?;
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
        if parsed.duel.is_some() {
            warn_retired_duel_config(config_path);
        }

        Ok(Self {
            execution_env: ExecutionEnvPolicy::from_snapshot(&snapshot),
            codex_execution: CodexExecutionPolicy::from_snapshot(&snapshot),
            persistence,
            pr: PrConfig {
                task_url_template: snapshot.pr_task_url_template.clone(),
            },
            scoring_enabled: snapshot.scoring_enabled,
            workflow_base_branch: snapshot.workflow_base_branch.clone(),
            workflow_auto_ship: snapshot.workflow_auto_ship,
            routines_source: snapshot.routines_role.as_deref() == Some("source"),
            crews,
            default_crew: snapshot.workflow_default_crew.clone(),
            system_crew: snapshot.workflow_system_crew.clone(),
            tasks_id_start: snapshot.tasks_id_start,
            snapshot,
        })
    }

    /// Configured `[tasks] id_start` floor, if any.
    pub(crate) fn tasks_id_start(&self) -> Option<u32> {
        self.tasks_id_start
    }

    pub(crate) fn workflow_base_branch(&self) -> &str {
        &self.workflow_base_branch
    }

    pub(crate) fn workflow_auto_ship(&self) -> bool {
        self.workflow_auto_ship
    }

    /// Configured crew for system activities. Resolution of the named crew is
    /// deliberately deferred to dispatch so a bad system crew does not stop
    /// unrelated activity execution.
    pub(crate) fn system_crew(&self) -> &str {
        &self.system_crew
    }

    pub(crate) fn routines_source(&self) -> bool {
        self.routines_source
    }

    pub(crate) fn pr_config(&self) -> &PrConfig {
        &self.pr
    }
}

pub fn load_effective_config(
    global_root: &Path,
    workspace_root: &Path,
) -> Result<EffectiveConfig, OrbitError> {
    let loaded = load_layered_runtime(global_root, workspace_root)?;
    let values = effective_values(
        &loaded.runtime,
        loaded.global.as_ref(),
        loaded.workspace.as_ref(),
    );
    Ok(EffectiveConfig {
        snapshot: loaded.runtime.snapshot,
        values,
    })
}

struct ConfigDocument {
    path: PathBuf,
    value: toml::Value,
}

struct LoadedRuntimeConfig {
    runtime: RuntimeConfig,
    global: Option<ConfigDocument>,
    workspace: Option<ConfigDocument>,
}

fn load_layered_runtime(
    global_root: &Path,
    workspace_root: &Path,
) -> Result<LoadedRuntimeConfig, OrbitError> {
    let global = read_config_document(&global_root.join("config.toml"))?;
    let workspace = if workspace_root != global_root {
        read_config_document(&workspace_root.join("config.toml"))?
    } else {
        None
    };
    let persistence = PersistenceConfig::default_for_roots(global_root, workspace_root);

    if global.is_none() && workspace.is_none() {
        return Ok(LoadedRuntimeConfig {
            runtime: RuntimeConfig {
                persistence,
                ..RuntimeConfig::default_for_data_root(global_root)
            },
            global,
            workspace,
        });
    }

    let mut merged = global
        .as_ref()
        .map(|document| document.value.clone())
        .unwrap_or_else(empty_document);
    if let Some(workspace_document) = &workspace {
        merge_tables(&mut merged, &workspace_document.value);

        // Registry table values are one config key, so a workspace value
        // replaces the global table rather than merging its members.
        // Dynamically named crews are intentionally excluded:
        // their fields layer recursively so one crew field can be overridden.
        for descriptor in super::registry::CONFIG_KEY_REGISTRY {
            if let Some(value) = value_at_path(&workspace_document.value, descriptor.key) {
                set_value_at_path(&mut merged, descriptor.key, value.clone());
            }
        }
        for key in WORKSPACE_REPLACE_ONLY_KEYS {
            if value_at_path(&workspace_document.value, key).is_none() {
                remove_value_at_path(&mut merged, key);
            }
        }
    }

    let config_path = workspace
        .as_ref()
        .or(global.as_ref())
        .map(|document| document.path.as_path())
        .unwrap_or_else(|| Path::new("<built-in defaults>"));
    let merged_raw = toml::to_string(&merged).map_err(|err| {
        OrbitError::InvalidInput(format!(
            "failed to render layered runtime config '{}': {err}",
            redact_home_dir(&config_path.display().to_string())
        ))
    })?;
    let runtime = RuntimeConfig::from_raw_str(&merged_raw, config_path, persistence)?;
    Ok(LoadedRuntimeConfig {
        runtime,
        global,
        workspace,
    })
}

fn read_config_document(path: &Path) -> Result<Option<ConfigDocument>, OrbitError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|err| {
        OrbitError::Io(format!(
            "failed to read runtime config '{}': {err}",
            redact_home_dir(&path.display().to_string())
        ))
    })?;
    let value = toml::from_str(&raw).map_err(|err| {
        OrbitError::InvalidInput(format!(
            "invalid runtime config '{}': {err}",
            redact_home_dir(&path.display().to_string())
        ))
    })?;
    Ok(Some(ConfigDocument {
        path: path.to_path_buf(),
        value,
    }))
}

fn empty_document() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn merge_tables(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) => merge_tables(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn value_at_path<'a>(document: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    let mut value = document;
    for segment in key.split('.') {
        value = value.as_table()?.get(segment)?;
    }
    Some(value)
}

fn crew_entry<'a>(
    document: &'a toml::Value,
    name: &str,
) -> Option<&'a toml::map::Map<String, toml::Value>> {
    document
        .as_table()?
        .get("crews")?
        .as_table()?
        .get(name)?
        .as_table()
}

fn set_value_at_path(document: &mut toml::Value, key: &str, value: toml::Value) {
    let segments = key.split('.').collect::<Vec<_>>();
    let Some((last, ancestors)) = segments.split_last() else {
        return;
    };
    let mut table = document.as_table_mut();
    for segment in ancestors {
        let Some(current) = table else {
            return;
        };
        let entry = current
            .entry((*segment).to_string())
            .or_insert_with(empty_document);
        table = entry.as_table_mut();
    }
    if let Some(table) = table {
        table.insert((*last).to_string(), value);
    }
}

fn remove_value_at_path(document: &mut toml::Value, key: &str) {
    let segments = key.split('.').collect::<Vec<_>>();
    let Some((last, ancestors)) = segments.split_last() else {
        return;
    };
    let mut value = document;
    for segment in ancestors {
        let Some(next) = value
            .as_table_mut()
            .and_then(|table| table.get_mut(*segment))
        else {
            return;
        };
        value = next;
    }
    if let Some(table) = value.as_table_mut() {
        table.remove(*last);
    }
}

fn effective_values(
    runtime: &RuntimeConfig,
    global: Option<&ConfigDocument>,
    workspace: Option<&ConfigDocument>,
) -> Vec<EffectiveConfigValue> {
    let mut values = runtime
        .snapshot
        .all_values()
        .into_iter()
        .map(|(key, value)| EffectiveConfigValue {
            key: key.to_string(),
            value,
            source: source_for_key(key, global, workspace),
        })
        .collect::<Vec<_>>();
    values.push(EffectiveConfigValue {
        key: "execution.env.inherit".to_string(),
        value: serde_json::json!(runtime.snapshot.execution_env_inherit),
        source: built_in_source(),
    });

    for (name, crew) in &runtime.crews {
        for (field, value) in [
            ("model", serde_json::json!(crew.assignment.model)),
            ("provider", serde_json::json!(crew.assignment.provider)),
            ("description", serde_json::json!(crew.description)),
            ("tags", serde_json::json!(crew.tags)),
        ] {
            let key = format!("crews.{name}.{field}");
            values.push(EffectiveConfigValue {
                source: source_for_crew_field(name, field, global, workspace),
                key,
                value,
            });
        }
    }
    values.sort_by(|left, right| left.key.cmp(&right.key));
    values
}

fn source_for_key(
    key: &str,
    global: Option<&ConfigDocument>,
    workspace: Option<&ConfigDocument>,
) -> ConfigValueSource {
    if let Some(document) = workspace
        && value_at_path(&document.value, key).is_some()
    {
        return file_source(ConfigValueSourceKind::Workspace, &document.path);
    }
    if workspace.is_some() && WORKSPACE_REPLACE_ONLY_KEYS.contains(&key) {
        return built_in_source();
    }
    if let Some(document) = global
        && value_at_path(&document.value, key).is_some()
    {
        return file_source(ConfigValueSourceKind::Global, &document.path);
    }
    if key == "workflow.default_crew"
        && std::env::var("CONSTELLATION_DEFAULT_PROVIDER")
            .is_ok_and(|value| !value.trim().is_empty())
    {
        return ConfigValueSource {
            kind: ConfigValueSourceKind::Environment,
            path: None,
        };
    }
    built_in_source()
}

fn source_for_crew_field(
    crew: &str,
    field: &str,
    global: Option<&ConfigDocument>,
    workspace: Option<&ConfigDocument>,
) -> ConfigValueSource {
    for (kind, document) in [
        (ConfigValueSourceKind::Workspace, workspace),
        (ConfigValueSourceKind::Global, global),
    ] {
        if let Some(document) = document
            && let Some(entry) = crew_entry(&document.value, crew)
            && entry.contains_key(field)
        {
            return file_source(kind, &document.path);
        }
    }
    built_in_source()
}

fn file_source(kind: ConfigValueSourceKind, path: &Path) -> ConfigValueSource {
    ConfigValueSource {
        kind,
        path: Some(path.to_path_buf()),
    }
}

fn built_in_source() -> ConfigValueSource {
    ConfigValueSource {
        kind: ConfigValueSourceKind::BuiltIn,
        path: None,
    }
}

pub(crate) fn default_crews() -> BTreeMap<String, Crew> {
    let mut crews = BTreeMap::new();
    for (name, model, provider) in [
        ("opus", CLAUDE_DEFAULT_STRONG, "claude"),
        ("sonnet", CLAUDE_DEFAULT_WEAK, "claude"),
        ("fable", CLAUDE_FABLE_MODEL, "claude"),
        ("sol", CODEX_SOL_MODEL, "codex"),
        ("terra", CODEX_TERRA_MODEL, "codex"),
        ("luna", CODEX_LUNA_MODEL, "codex"),
        ("gemini", GEMINI_CREW_MODEL, "gemini"),
        ("grok", GROK_DEFAULT_MODEL, "grok"),
    ] {
        crews.insert(
            name.to_string(),
            Crew {
                name: name.to_string(),
                assignment: crew_assignment(model, provider),
                description: None,
                tags: Vec::new(),
            },
        );
    }
    crews
}

fn crew_assignment(model: &str, provider: &str) -> CrewAssignment {
    CrewAssignment {
        model: model.to_string(),
        provider: provider.to_string(),
    }
}

/// The retired invocation-level agent backend override.
pub(super) const RETIRED_BACKEND_ENV: &str = "ORBIT_BACKEND";

/// [ORB-10801] `ORBIT_BACKEND` and `[runtime] backend` were tiers 2 and 3 of
/// the retired agent-loop backend precedence chain. Both are refused rather
/// than ignored: an operator who still pins `http` must be told their runs are
/// now CLI-agent runs instead of having that substitution made for them.
/// `cli` named the surviving path, so it stays accepted and inert.
fn reject_retired_backend_overrides(
    document: &toml::Value,
    env_value: Option<&str>,
) -> Result<(), OrbitError> {
    if let Some(raw) = env_value.map(str::trim).filter(|value| !value.is_empty()) {
        check_retired_backend_value(raw).map_err(|error| {
            OrbitError::InvalidInput(format!("{RETIRED_BACKEND_ENV} is retired: {error}"))
        })?;
    }
    let Some(value) = value_at_path(document, "runtime.backend") else {
        return Ok(());
    };
    let raw = value.as_str().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "[runtime] backend must be a string; {RETIRED_BACKEND_MIGRATION}"
        ))
    })?;
    check_retired_backend_value(raw)
        .map_err(|error| OrbitError::InvalidInput(format!("[runtime] {error}")))
}

#[cfg(test)]
pub(super) fn retired_backend_override_check(
    document: &toml::Value,
    env_value: Option<&str>,
) -> Result<(), OrbitError> {
    reject_retired_backend_overrides(document, env_value)
}

fn reject_stale_agent_tables(
    raw: Option<&BTreeMap<String, toml::Value>>,
) -> Result<(), OrbitError> {
    if raw.is_some() {
        // ORB-00058: source provenance for retiring the old agent-role schema.
        return Err(OrbitError::InvalidInput(
            "config schema no longer supports [agent.<role>] tables; migrate to [crews.<name>] with [workflow].default_crew".to_string(),
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

fn crew_assignment_from_raw(crew: &str, raw: &RawCrewEntry) -> Result<CrewAssignment, OrbitError> {
    let has_legacy = raw.planner.is_some() || raw.implementer.is_some() || raw.reviewer.is_some();
    if has_legacy {
        return Err(OrbitError::InvalidInput(format!(
            "[crews.{crew}] uses retired planner/implementer/reviewer role tables; rewrite it with flat `model` and `provider` fields only"
        )));
    }
    reject_retired_crew_backend(crew, raw.backend.as_deref())?;
    Ok(CrewAssignment {
        model: required_crew_field(crew, "model", raw.model.as_deref())?,
        provider: required_crew_field(crew, "provider", raw.provider.as_deref())?,
    })
}

/// [ORB-10801] `[crews.<name>] backend` selected the agent execution backend.
/// Only the CLI agent path survives, so `cli` stays accepted and inert while
/// the removed values are refused: remapping `http` onto the CLI agent would
/// change which runtime the crew dispatches to without saying so.
fn reject_retired_crew_backend(crew: &str, raw: Option<&str>) -> Result<(), OrbitError> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    check_retired_backend_value(value)
        .map_err(|error| OrbitError::InvalidInput(format!("[crews.{crew}] {error}")))
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

pub(super) const RETIRED_DUEL_CONFIG_WARNING: &str =
    "[duel] and [duel.models] are retired and ignored; remove both keys from config.toml";

fn warn_retired_duel_config(config_path: &Path) {
    let path = redact_home_dir(&config_path.display().to_string());
    tracing::warn!(
        config = %path,
        RETIRED_DUEL_CONFIG_WARNING,
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
