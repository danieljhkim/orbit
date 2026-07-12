//! Static registry of settable `config.toml` keys.
//!
//! This is the single source of truth for which dotted TOML paths `orbit
//! config get`/`set` accept. It powers three things: `orbit config keys`,
//! rejecting unknown keys on `get`/`set` with a "did you mean" hint, and the
//! `settings:` section of `orbit config show`.
//!
//! Deliberately **not** included here: derived/read-only values
//! (`global_root`, persistence paths, the resolved config path itself — see
//! `ConfigSnapshot`'s `derived` fields, which are surfaced by `show` but are
//! never settable) and dynamically-named tables (`[crews.<name>]`,
//! `[duel.models]` per-entry overrides) whose key space isn't a fixed set of
//! dotted paths. Those remain hand-editable in `config.toml` directly; only
//! the fixed, well-known scalar/array leaves get a registry entry.

/// One entry in the static key registry: a dotted `config.toml` path plus
/// the metadata `orbit config keys` and error messages need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigKeyDescriptor {
    /// Dotted TOML path, e.g. `"workflow.base_branch"`.
    pub key: &'static str,
    /// Human-readable type, e.g. `"bool"`, `"string"`, `"array<string>"`.
    pub value_type: &'static str,
    /// One-line description shown by `orbit config keys`.
    pub description: &'static str,
}

/// All settable keys, sorted by dotted path.
pub const CONFIG_KEY_REGISTRY: &[ConfigKeyDescriptor] = &[
    ConfigKeyDescriptor {
        key: "duel.candidates",
        value_type: "array<string>",
        description: "Agent families eligible as planning-duel candidates (at least 3, from the known agent family set).",
    },
    ConfigKeyDescriptor {
        key: "duel.models",
        value_type: "table<string, string>",
        description: "Per-family model override for planning-duel candidates, e.g. { codex = \"<model-id>\" }.",
    },
    ConfigKeyDescriptor {
        key: "execution.codex.approval_policy",
        value_type: "string",
        description: "Codex approval policy: one of untrusted, on-request, never.",
    },
    ConfigKeyDescriptor {
        key: "execution.codex.sandbox",
        value_type: "string",
        description: "Codex sandbox mode: one of read-only, workspace-write, danger-full-access.",
    },
    ConfigKeyDescriptor {
        key: "execution.env.pass",
        value_type: "array<string>",
        description: "Environment variable names allow-listed for passthrough into agent subprocesses.",
    },
    ConfigKeyDescriptor {
        key: "graph.editing",
        value_type: "bool",
        description: "Whether the code graph editing surface is enabled.",
    },
    ConfigKeyDescriptor {
        key: "pr.task_url_template",
        value_type: "string",
        description: "URL template used to link a task ID in PR descriptions.",
    },
    ConfigKeyDescriptor {
        key: "routines.role",
        value_type: "string",
        description: "Opt-in for the routine scheduler; the only supported value is 'source' (marks this workspace as a routine source for `orbit sweep`).",
    },
    ConfigKeyDescriptor {
        key: "runtime.backend",
        value_type: "string",
        description: "Default v2 agent_loop execution backend: one of http, cli, auto.",
    },
    ConfigKeyDescriptor {
        key: "runtime.log_max_file_mb",
        value_type: "integer",
        description: "Roll the active JSONL log once it grows past this many MiB (must be >= 1 and <= runtime.log_max_total_mb).",
    },
    ConfigKeyDescriptor {
        key: "runtime.log_max_total_mb",
        value_type: "integer",
        description: "Total size budget (MiB) across JSONL log archives; oldest are pruned first when exceeded (must be >= 1).",
    },
    ConfigKeyDescriptor {
        key: "runtime.log_retention_days",
        value_type: "integer",
        description: "Delete JSONL log archives whose mtime is older than this many days (must be >= 1).",
    },
    ConfigKeyDescriptor {
        key: "scoring.enabled",
        value_type: "bool",
        description: "Whether scoreboard metrics are recorded for task runs.",
    },
    ConfigKeyDescriptor {
        key: "task.approval.delegate_approval",
        value_type: "bool",
        description: "Whether task approval can be delegated to another agent.",
    },
    ConfigKeyDescriptor {
        key: "task.approval.required_for_agent",
        value_type: "bool",
        description: "Whether agent-initiated tasks require human approval before running.",
    },
    ConfigKeyDescriptor {
        key: "tasks.id_start",
        value_type: "integer",
        description: "Floor for the local task-id allocator on this machine (forward-only; lets machines hold disjoint id ranges). Capped by ORB_TASK_ID_MAX.",
    },
    ConfigKeyDescriptor {
        key: "workflow.auto_ship",
        value_type: "bool",
        description: "Opt-in for unattended ship dispatch via the routine/sweep scheduler.",
    },
    ConfigKeyDescriptor {
        key: "workflow.base_branch",
        value_type: "string",
        description: "Default base branch for ship and duel-plan workflows.",
    },
    ConfigKeyDescriptor {
        key: "workflow.default_crew",
        value_type: "string",
        description: "Named crew used when a task does not declare `crew` and no CLI override is given.",
    },
    ConfigKeyDescriptor {
        key: "worktree.gc_failed_retention_days",
        value_type: "integer",
        description: "Days a failed/timeout/interrupted run's worktree is kept for debugging before `orbit run gc` reclaims it (must be >= 0); success/cancelled reap immediately.",
    },
];

/// Look up a key's descriptor, or `None` if it isn't in the registry.
pub fn describe(key: &str) -> Option<&'static ConfigKeyDescriptor> {
    CONFIG_KEY_REGISTRY.iter().find(|entry| entry.key == key)
}

/// All registered dotted key paths, sorted. Used both for `orbit config
/// keys` and as the "did you mean" candidate list on an unknown key,
/// following the `resolve_crew` convention in
/// `orbit_common::types::agent_pair`: the full valid set is passed as
/// candidates rather than a fuzzy-matched subset (this workspace has no
/// fuzzy-matching dependency, and this registry is small enough that the
/// full list is itself a useful hint).
pub fn all_key_names() -> Vec<String> {
    CONFIG_KEY_REGISTRY
        .iter()
        .map(|entry| entry.key.to_string())
        .collect()
}
