use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    pub(super) execution: Option<RawExecutionConfig>,
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) agents: Option<RawAgentModelsConfig>,
    pub(super) workflow: Option<RawWorkflowConfig>,
    pub(super) scoring: Option<RawScoringConfig>,
    pub(super) graph: Option<RawGraphConfig>,
    pub(super) watch: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawAgentModelsConfig {
    pub(super) claude: Option<RawAgentModelEntry>,
    pub(super) codex: Option<RawAgentModelEntry>,
    pub(super) gemini: Option<RawAgentModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawAgentModelEntry {
    pub(super) strong: String,
    pub(super) weak: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawWorkflowConfig {
    pub(super) ship: Option<RawShipWorkflowConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawShipWorkflowConfig {
    pub(super) plan: Option<RawAgentAssignment>,
    pub(super) implement: Option<RawAgentAssignment>,
    pub(super) review: Option<RawAgentAssignment>,
    pub(super) finalize: Option<RawAgentAssignment>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawAgentAssignment {
    pub(super) agent: String,
    pub(super) model: String,
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
