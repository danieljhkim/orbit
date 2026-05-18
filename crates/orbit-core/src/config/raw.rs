use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    pub(super) execution: Option<RawExecutionConfig>,
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) pr: Option<RawPrSection>,
    pub(super) scoring: Option<RawScoringConfig>,
    pub(super) graph: Option<RawGraphConfig>,
    pub(super) artifacts: Option<RawArtifactsConfig>,
    pub(super) knowledge: Option<RawKnowledgeConfig>,
    pub(super) watch: Option<toml::Value>,
    pub(super) runtime: Option<RawRuntimeSection>,
    pub(super) workflow: Option<RawWorkflowConfig>,
    pub(super) duel: Option<RawDuelSection>,
    /// Removed in ORB-00058. Kept only so config loading can reject stale
    /// `[agent.<role>]` tables with an explicit migration error.
    pub(super) agent: Option<BTreeMap<String, RawAgentRoleConfig>>,
    /// `[crews.<name>]` registry. Each table supplies the three role
    /// assignments Orbit resolves at task run start.
    pub(super) crews: Option<BTreeMap<String, RawCrewEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawWorkflowConfig {
    /// `workflow.base_branch` — repo-level default base branch for ship and
    /// duel-plan workflows. When absent, defaults to `agent-main`.
    /// Repos that ship directly from `main` can set this to `"main"`.
    pub(super) base_branch: Option<String>,
    /// Named crew used when a task does not declare `crew` and no CLI
    /// override is provided.
    pub(super) default_crew: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
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
pub(super) struct RawArtifactsConfig {
    pub(super) auto_publish: Option<bool>,
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
