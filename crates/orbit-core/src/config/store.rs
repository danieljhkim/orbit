//! Surgical, comment-preserving `config.toml` reads/writes for `orbit config`.
//!
//! [`ConfigStore`] wraps a single `config.toml` file as a `toml_edit::DocumentMut`
//! so `orbit config set` can edit one key without disturbing any other part of
//! the file's formatting or hand-written comments. Validation always goes
//! through [`RuntimeConfig::from_raw_str`] — the exact same pipeline
//! `RuntimeConfig::load_layered` uses at process startup — so a `set` can never
//! produce a document the runtime itself would reject.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value as JsonValue, json};
use toml_edit::{DocumentMut, Item, Table};

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::atomic_write_text;
use orbit_common::utility::redaction::redact_home_dir;

use super::persistence::PersistenceConfig;
use super::registry;
use super::runtime::RuntimeConfig;

/// Which physical `config.toml` file a [`ConfigStore`] is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Workspace,
}

impl ConfigScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}

/// How to initialize a workspace `config.toml` that doesn't exist yet, for
/// the first `orbit config set` write against it. Fail-closed by default: a
/// bare `set` (without `--global`) must never silently create a workspace
/// config that would newly shadow the global one under replace-not-merge
/// semantics (see the `config` module doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceInitMode {
    /// The workspace file must already exist; error out with a hint otherwise.
    RequireExisting,
    /// Seed the new workspace file from the current global file's content
    /// (or an empty document if no global file exists either).
    SeedFromGlobal,
    /// Start from an empty TOML document.
    Fresh,
}

/// An in-memory, surgically-editable view of one `config.toml` file, plus
/// the machinery to validate an edit and atomically persist it.
pub struct ConfigStore {
    scope: ConfigScope,
    path: PathBuf,
    doc: DocumentMut,
}

impl ConfigStore {
    /// Open `path` for the given scope, reading its current content if it
    /// exists on disk. A missing file is not an error: it opens as an empty
    /// document, matching how `RuntimeConfig::load_layered` treats a missing
    /// config (every setting falls back to its default).
    pub fn open(scope: ConfigScope, path: impl Into<PathBuf>) -> Result<Self, OrbitError> {
        let path = path.into();
        let content = read_optional(&path)?;
        Self::from_content(scope, path, &content)
    }

    /// Open the workspace `config.toml` for a `set`, applying the
    /// fail-closed first-write rule: when the workspace file does not yet
    /// exist, `mode` decides whether to seed it from the global file, start
    /// fresh, or refuse.
    pub fn open_for_workspace_set(
        workspace_config_path: impl Into<PathBuf>,
        global_config_path: &Path,
        mode: WorkspaceInitMode,
    ) -> Result<Self, OrbitError> {
        let path = workspace_config_path.into();
        if path.exists() {
            return Self::open(ConfigScope::Workspace, path);
        }
        let content = match mode {
            WorkspaceInitMode::RequireExisting => {
                return Err(OrbitError::invalid_input_with_suggestions(
                    format!(
                        "no workspace config exists yet at '{}'; `orbit config set` without \
                         --global refuses to create one implicitly, since that would newly \
                         shadow the global config. Rerun with --seed-from-global to copy the \
                         current global config as a starting point, or --fresh to start from an \
                         empty file",
                        redact_home_dir(&path.display().to_string())
                    ),
                    Vec::new(),
                ));
            }
            WorkspaceInitMode::SeedFromGlobal => read_optional(global_config_path)?,
            WorkspaceInitMode::Fresh => String::new(),
        };
        Self::from_content(ConfigScope::Workspace, path, &content)
    }

    fn from_content(scope: ConfigScope, path: PathBuf, content: &str) -> Result<Self, OrbitError> {
        let doc = content.parse::<DocumentMut>().map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid TOML in '{}': {err}",
                redact_home_dir(&path.display().to_string())
            ))
        })?;
        Ok(Self { scope, path, doc })
    }

    pub fn scope(&self) -> ConfigScope {
        self.scope
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists_on_disk(&self) -> bool {
        self.path.exists()
    }

    /// The fully resolved (defaulted) view of this document, as if it were
    /// loaded as the effective `config.toml`. Used by both `orbit config
    /// show` and `orbit config get` so they report identical values for the
    /// same scope.
    pub fn snapshot(&self) -> Result<ConfigSnapshot, OrbitError> {
        // Persistence paths are derived from the two data roots, not from
        // the config document, and are irrelevant to key validation here;
        // this is discarded by every caller of `snapshot()`.
        let persistence =
            PersistenceConfig::default_for_data_root(self.path.parent().unwrap_or(&self.path));
        let raw = self.doc.to_string();
        let runtime = RuntimeConfig::from_raw_str(&raw, &self.path, persistence)?;
        Ok(ConfigSnapshot::from(&runtime))
    }

    /// Look up the effective value of a single registry key.
    pub fn effective_value(&self, key: &str) -> Result<JsonValue, OrbitError> {
        require_known_key(key)?;
        let snapshot = self.snapshot()?;
        Ok(snapshot.value_for(key).unwrap_or(JsonValue::Null))
    }

    /// Set `key` to the TOML-literal-or-string parse of `raw_value`,
    /// mutating the in-memory document only. Callers must call
    /// [`Self::validate`] and then [`Self::save`] afterward — `set_value`
    /// never touches disk.
    pub fn set_value(&mut self, key: &str, raw_value: &str) -> Result<(), OrbitError> {
        require_known_key(key)?;
        let value = parse_value_literal(raw_value);
        let segments: Vec<&str> = key.split('.').collect();
        // `require_known_key` above already rejects `key` unless it matches
        // a non-empty registry entry, so `split_last` is always `Some` here;
        // handled as an error rather than `expect()` since this is
        // reachable from user input, not a purely local invariant.
        let (last, ancestors) = segments.split_last().ok_or_else(|| {
            OrbitError::InvalidInput(format!("config key '{key}' must not be empty"))
        })?;

        let mut table: &mut Table = self.doc.as_table_mut();
        for segment in ancestors {
            let item = table
                .entry(segment)
                .or_insert_with(|| Item::Table(Table::new()));
            table = item.as_table_mut().ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "cannot set '{key}': '{segment}' along its path is already a non-table value \
                     in '{}'",
                    redact_home_dir(&self.path.display().to_string())
                ))
            })?;
        }
        // Prefer mutating an existing key's `Item` in place over
        // `Table::insert`, which always constructs a brand-new `Key` node:
        // that would silently drop any full-line comment attached to the
        // existing key (its "decor") even though we're only replacing the
        // value.
        match table.get_mut(last) {
            Some(existing) => *existing = Item::Value(value),
            None => {
                table.insert(last, Item::Value(value));
            }
        }
        Ok(())
    }

    /// Run the in-memory document through the exact same
    /// `RawRuntimeConfig` → `RuntimeConfig` validation pipeline as
    /// `RuntimeConfig::load_layered`, without writing anything.
    pub fn validate(&self) -> Result<(), OrbitError> {
        self.snapshot().map(|_| ())
    }

    /// Atomically write the current in-memory document to `self.path`
    /// (temp file + rename, via `orbit_common::utility::fs::atomic_write_text`).
    /// Callers should call [`Self::validate`] first: `save` does not
    /// validate on its own.
    pub fn save(&self) -> Result<(), OrbitError> {
        atomic_write_text(&self.path, &self.doc.to_string()).map_err(|err| {
            OrbitError::Io(format!(
                "failed to write config '{}': {err}",
                redact_home_dir(&self.path.display().to_string())
            ))
        })
    }
}

fn require_known_key(key: &str) -> Result<(), OrbitError> {
    if registry::describe(key).is_some() {
        Ok(())
    } else {
        Err(OrbitError::invalid_input_with_suggestions(
            format!("unknown config key '{key}'"),
            registry::all_key_names(),
        ))
    }
}

fn read_optional(path: &Path) -> Result<String, OrbitError> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(OrbitError::Io(format!(
            "failed to read config '{}': {err}",
            redact_home_dir(&path.display().to_string())
        ))),
    }
}

/// Parse `raw` as a TOML literal (bool/int/float/array/inline-table/etc),
/// falling back to a plain string if it doesn't parse as one. No `--type`
/// flag: this is the entire type-inference rule for `orbit config set`.
///
/// `toml_edit::Value` (like `toml::Value`) has no public "parse a bare
/// value" entry point — both crates' string parsers expect a full
/// `key = value` document, so `"10000".parse::<toml_edit::Value>()` is not
/// a thing. Instead, wrap `raw` as the value of a throwaway key in a
/// synthetic one-line document, parse *that*, and pull the value back out.
/// Any extra keys `raw` might smuggle in (e.g. `1\nother = "x"`) are parsed
/// but never read, so this can't be used to inject unrelated keys — worst
/// case a crafted `raw` just fails to parse and falls back to a string.
fn parse_value_literal(raw: &str) -> toml_edit::Value {
    const SCRATCH_KEY: &str = "_orbit_config_set_value";
    let synthetic = format!("{SCRATCH_KEY} = {raw}");
    synthetic
        .parse::<DocumentMut>()
        .ok()
        .and_then(|doc| doc.get(SCRATCH_KEY).and_then(Item::as_value).cloned())
        .unwrap_or_else(|| toml_edit::Value::from(raw))
}

/// Fully resolved (defaulted) view of one `config.toml` file's settable
/// keys, projected from [`RuntimeConfig`] so `orbit config show`/`get`
/// report the same values the runtime itself would use.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub execution_env_inherit: bool,
    pub execution_env_pass: Vec<String>,
    pub codex_sandbox: String,
    pub codex_approval_policy: Option<String>,
    pub task_approval_required_for_agent: bool,
    pub task_delegate_approval: bool,
    pub scoring_enabled: bool,
    pub graph_editing: bool,
    pub runtime_backend: Option<String>,
    /// Effective JSONL log retention window in days [ORB-00415]. Always
    /// populated: falls back to the `LogRotationConfig` default when the key
    /// is not explicitly set.
    pub runtime_log_retention_days: u64,
    /// Effective total-size budget across JSONL log archives, in MiB.
    pub runtime_log_max_total_mb: u64,
    /// Effective per-file roll threshold for the active JSONL log, in MiB.
    pub runtime_log_max_file_mb: u64,
    pub workflow_base_branch: String,
    pub workflow_default_crew: Option<String>,
    pub workflow_auto_ship: bool,
    pub routines_source: bool,
    pub worktree_gc_success_retention_days: u64,
    pub worktree_gc_failure_retention_days: u64,
    pub tasks_id_start: Option<u32>,
    pub duel_candidates: Vec<String>,
    pub duel_models: std::collections::BTreeMap<String, String>,
    pub pr_task_url_template: Option<String>,
}

impl From<&RuntimeConfig> for ConfigSnapshot {
    fn from(config: &RuntimeConfig) -> Self {
        // The resolved `LogRotationConfig` stores byte budgets that were
        // constructed by multiplying the raw MiB inputs by `BYTES_PER_MB`
        // (or the analogous default constants), so integer division here
        // recovers the exact MiB values the user set or would set.
        const BYTES_PER_MB: u64 = 1024 * 1024;
        let log_rotation = config.log_rotation();
        Self {
            execution_env_inherit: config.execution_env.inherit(),
            execution_env_pass: config.execution_env.pass().to_vec(),
            codex_sandbox: config.codex_execution.sandbox().to_string(),
            codex_approval_policy: config
                .codex_execution
                .approval_policy()
                .map(ToString::to_string),
            task_approval_required_for_agent: config.task_approval.required_for_agent,
            task_delegate_approval: config.task_approval.delegate_approval,
            scoring_enabled: config.scoring_enabled,
            graph_editing: config.graph_editing,
            runtime_backend: config.v2_backend().map(ToString::to_string),
            runtime_log_retention_days: log_rotation.retention_days,
            runtime_log_max_total_mb: log_rotation.max_total_bytes / BYTES_PER_MB,
            runtime_log_max_file_mb: log_rotation.max_file_bytes / BYTES_PER_MB,
            workflow_base_branch: config.workflow_base_branch().to_string(),
            workflow_default_crew: config.default_crew.clone(),
            workflow_auto_ship: config.workflow_auto_ship(),
            routines_source: config.routines_source(),
            worktree_gc_success_retention_days: config.worktree_gc_success_retention_days(),
            worktree_gc_failure_retention_days: config.worktree_gc_failure_retention_days(),
            tasks_id_start: config.tasks_id_start(),
            duel_candidates: config.duel_config().candidates.clone(),
            duel_models: config.duel_config().models.clone(),
            pr_task_url_template: config.pr_config().task_url_template.clone(),
        }
    }
}

impl ConfigSnapshot {
    /// Look up the value for a registry key. Returns `None` for a key the
    /// registry doesn't know about; callers validate the key against the
    /// registry before calling this.
    pub fn value_for(&self, key: &str) -> Option<JsonValue> {
        Some(match key {
            "duel.candidates" => json!(self.duel_candidates),
            "duel.models" => json!(self.duel_models),
            "execution.codex.approval_policy" => json!(self.codex_approval_policy),
            "execution.codex.sandbox" => json!(self.codex_sandbox),
            "execution.env.pass" => json!(self.execution_env_pass),
            "graph.editing" => json!(self.graph_editing),
            "gc.worktrees.failure_retention_days" => {
                json!(self.worktree_gc_failure_retention_days)
            }
            "gc.worktrees.success_retention_days" => {
                json!(self.worktree_gc_success_retention_days)
            }
            "pr.task_url_template" => json!(self.pr_task_url_template),
            "routines.role" => json!(if self.routines_source {
                Some("source")
            } else {
                None
            }),
            "runtime.backend" => json!(self.runtime_backend),
            "runtime.log_max_file_mb" => json!(self.runtime_log_max_file_mb),
            "runtime.log_max_total_mb" => json!(self.runtime_log_max_total_mb),
            "runtime.log_retention_days" => json!(self.runtime_log_retention_days),
            "scoring.enabled" => json!(self.scoring_enabled),
            "task.approval.delegate_approval" => json!(self.task_delegate_approval),
            "task.approval.required_for_agent" => json!(self.task_approval_required_for_agent),
            "tasks.id_start" => json!(self.tasks_id_start),
            "workflow.auto_ship" => json!(self.workflow_auto_ship),
            "workflow.base_branch" => json!(self.workflow_base_branch),
            "workflow.default_crew" => json!(self.workflow_default_crew),
            _ => return None,
        })
    }

    /// All registry keys paired with their resolved value, in registry
    /// order (sorted by key) — the data behind `orbit config show`'s
    /// `settings:` section.
    pub fn all_values(&self) -> Vec<(&'static str, JsonValue)> {
        registry::CONFIG_KEY_REGISTRY
            .iter()
            .filter_map(|entry| self.value_for(entry.key).map(|value| (entry.key, value)))
            .collect()
    }
}
