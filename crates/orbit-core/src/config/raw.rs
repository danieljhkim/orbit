use serde::Deserialize;

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
