use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    pub(super) execution: Option<RawExecutionConfig>,
    pub(super) user: Option<RawUserSection>,
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) job: Option<RawEntitySection>,
    pub(super) activity: Option<RawEntitySection>,
    pub(super) skill: Option<RawEntitySection>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) watch: Option<RawEntitySection>,
    pub(super) audit: Option<RawEntitySection>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawExecutionConfig {
    pub(super) env: Option<RawExecutionEnvConfig>,
    pub(super) codex: Option<RawCodexExecutionConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct RawUserSection {
    pub(super) name: Option<String>,
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
pub(super) struct RawEntitySection {
    pub(super) persistence: Option<RawPersistenceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskSection {
    pub(super) persistence: Option<RawPersistenceConfig>,
    pub(super) approval: Option<RawTaskApprovalConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskApprovalConfig {
    pub(super) required_for_agent: Option<bool>,
    pub(super) delegate_approval: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawPersistenceConfig {
    #[serde(rename = "type")]
    pub(super) persistence_type: Option<String>,
    pub(super) path: Option<String>,
    pub(super) format: Option<String>,
}
