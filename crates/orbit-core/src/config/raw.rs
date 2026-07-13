use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    pub(super) execution: Option<RawExecutionConfig>,
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) tasks: Option<RawTasksConfig>,
    pub(super) pr: Option<RawPrSection>,
    pub(super) scoring: Option<RawScoringConfig>,
    pub(super) graph: Option<RawGraphConfig>,
    pub(super) knowledge: Option<RawKnowledgeConfig>,
    pub(super) watch: Option<toml::Value>,
    pub(super) runtime: Option<RawRuntimeSection>,
    pub(super) workflow: Option<RawWorkflowConfig>,
    pub(super) gc: Option<RawGcConfig>,
    pub(super) routines: Option<RawRoutinesConfig>,
    pub(super) duel: Option<RawDuelSection>,
    /// Removed in ORB-00058. Kept only so config loading can reject stale
    /// `[agent.<role>]` tables with an explicit migration error.
    pub(super) agent: Option<BTreeMap<String, RawAgentRoleConfig>>,
    /// `[crews.<name>]` registry. Each table supplies one assignment Orbit
    /// resolves for every activity role at task run start.
    pub(super) crews: Option<BTreeMap<String, RawCrewEntry>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(super) struct RawGcConfig {
    pub(super) worktrees: Option<RawWorktreeGcConfig>,
    pub(super) runs: Option<RawRunGcConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(super) struct RawRunGcConfig {
    pub(super) archive_after_days: Option<u64>,
    pub(super) purge_after_days: Option<u64>,
    pub(super) failure_archive_after_days: Option<u64>,
    pub(super) failure_purge_after_days: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(super) struct RawWorktreeGcConfig {
    /// Days to retain successful/cancelled worktrees after the persisted
    /// terminal transition. Zero permits immediate collection.
    pub(super) success_retention_days: Option<u64>,
    /// Days to retain failed/timeout/interrupted worktrees after the persisted
    /// terminal transition. Resumable interrupted worktrees remain protected.
    pub(super) failure_retention_days: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct RawWorkflowConfig {
    /// `workflow.base_branch` — repo-level default base branch for ship and
    /// duel-plan workflows. When absent, defaults to `main`.
    /// Repos that keep an `agent-main` buffer branch set this to
    /// `"agent-main"`.
    pub(super) base_branch: Option<String>,
    /// `workflow.auto_ship` — opt-in for unattended ship dispatch
    /// (`orbit run ship-sweep` and other schedulers). When absent or
    /// `false`, sweeps skip this workspace.
    pub(super) auto_ship: Option<bool>,
    /// Named crew used when a task does not declare `crew` and no CLI
    /// override is provided.
    pub(super) default_crew: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRoutinesConfig {
    /// `routines.role` — opt-in for the routine scheduler. `"source"` marks
    /// this workspace as a routine source: `orbit sweep` loads
    /// `.orbit/routines/*.yaml` from it. Any other value is a config error
    /// (fail-closed; scheduled execution must never be enabled by a typo'd
    /// key that silently parses).
    pub(super) role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTasksConfig {
    /// `tasks.id_start` — floor for the local task-id allocator on this machine.
    /// On runtime build the allocator is raised to at least this value (never
    /// lowered), so machines can be handed disjoint id ranges (e.g. one 0–9999,
    /// another 10000+) to avoid cross-machine collisions. Capped by
    /// `ORB_TASK_ID_MAX`; setting it near the ceiling shrinks the usable range.
    pub(super) id_start: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct RawDuelSection {
    pub(super) candidates: Option<Vec<String>>,
    pub(super) models: Option<BTreeMap<String, String>>,
}

/// Schema for a single role assignment in `[crews.<name>]`.
///
/// Serialize is derived so the writer in `bootstrap` can emit fresh entries
/// without hand-rolling TOML. The struct is `pub` so the CLI can hand a map
/// of these directly into `InitOptions::role_settings` when running
/// interactive prompts.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawAgentRoleConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawCrewEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Legacy three-role fields retained for compatibility. Runtime loading
    /// selects `implementer` and warns when the discarded roles diverge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<RawAgentRoleConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementer: Option<RawAgentRoleConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<RawAgentRoleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawKnowledgeConfig {
    /// Deprecated legacy key. Kept only so loaders can warn and ignore it.
    pub(super) task_id_pattern: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeSection {
    /// `runtime.backend` — persisted default for the v2 `agent_loop` execution
    /// backend (§3.1). One of `http`, `cli`, `auto`; validated by
    /// `RuntimeConfig::load_layered`.
    pub(super) backend: Option<String>,
    /// `runtime.log_retention_days` — delete JSONL log archives older than this
    /// many days. [ORB-00415] Validated by `RuntimeConfig::load_layered`.
    pub(super) log_retention_days: Option<u64>,
    /// `runtime.log_max_total_mb` — total size budget (MiB) across JSONL log
    /// archives; oldest are pruned first when exceeded.
    pub(super) log_max_total_mb: Option<u64>,
    /// `runtime.log_max_file_mb` — roll the active JSONL log once it grows past
    /// this many MiB.
    pub(super) log_max_file_mb: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawGraphConfig {
    pub(super) editing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawScoringConfig {
    pub(super) enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawExecutionConfig {
    pub(super) env: Option<RawExecutionEnvConfig>,
    pub(super) codex: Option<RawCodexExecutionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawExecutionEnvConfig {
    // `inherit` is intentionally not a field: env inheritance is not
    // configurable. Agent subprocesses always run with a cleared environment
    // plus the `pass` allowlist, never the orbit process's full environment.
    // A stale `inherit = ...` key in an existing config.toml is silently
    // ignored (no `deny_unknown_fields`). See ORB-00365.
    pub(super) pass: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawCodexExecutionConfig {
    pub(super) sandbox: Option<String>,
    pub(super) approval_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskSection {
    pub(super) approval: Option<RawTaskApprovalConfig>,
    /// Removed pre-release selector. Kept here only so config loading can
    /// reject stale keys with an explicit task-artifacts cutover message.
    pub(super) artifact_store: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawPrSection {
    pub(super) task_url_template: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskApprovalConfig {
    pub(super) required_for_agent: Option<bool>,
    pub(super) delegate_approval: Option<bool>,
}
