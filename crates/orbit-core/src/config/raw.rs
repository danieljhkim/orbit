use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    pub(super) execution: Option<RawExecutionConfig>,
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) scoring: Option<RawScoringConfig>,
    pub(super) graph: Option<RawGraphConfig>,
    pub(super) knowledge: Option<RawKnowledgeConfig>,
    pub(super) watch: Option<toml::Value>,
    pub(super) runtime: Option<RawRuntimeSection>,
    /// `[agent.<role>]` tables, e.g. `[agent.reviewer]`. Keys are role names
    /// (`reviewer`, `implementer`, `planner`, or any free-form string the
    /// resolver chooses to honour). Values supply optional `provider`,
    /// `model`, and `backend` overrides per role. Written by `orbit init`
    /// from interactive prompts (T20260428-9). The consumer side (resolver,
    /// dispatch override) is the follow-up T20260428-12 — until that lands,
    /// the field is parsed but unread.
    #[allow(dead_code)]
    pub(super) agent: Option<BTreeMap<String, RawAgentRoleConfig>>,
}

/// Schema for a single `[agent.<role>]` table in `config.toml`. All fields
/// are optional so partial overrides round-trip cleanly.
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

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawKnowledgeConfig {
    /// `knowledge.task_id_pattern` — workspace override for the task-ID
    /// extraction regex used by `orbit graph build` and `orbit graph history`.
    /// `None` falls back to the Orbit default.
    pub(super) task_id_pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeSection {
    /// `runtime.backend` — persisted default for the v2 `agent_loop` execution
    /// backend (§3.1). One of `http`, `cli`, `auto`.
    pub(super) backend: Option<String>,
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
    pub(super) inherit: Option<bool>,
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
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskApprovalConfig {
    pub(super) required_for_agent: Option<bool>,
    pub(super) delegate_approval: Option<bool>,
}
