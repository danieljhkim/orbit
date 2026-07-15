use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) knowledge: Option<RawKnowledgeConfig>,
    pub(super) watch: Option<toml::Value>,
    /// Removed in ORB-00058. Kept only so config loading can reject stale
    /// `[agent.<role>]` tables with an explicit migration error.
    pub(super) agent: Option<BTreeMap<String, RawAgentRoleConfig>>,
    /// `[crews.<name>]` registry. Each table supplies one assignment Orbit
    /// resolves for every activity role at task run start.
    pub(super) crews: Option<BTreeMap<String, RawCrewEntry>>,
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

/// Bootstrap writer shape for the fixed planning-duel section. Runtime
/// admission reads these keys through the registry table.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct RawDuelSection {
    pub(super) candidates: Option<Vec<String>>,
    pub(super) models: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawKnowledgeConfig {
    /// Deprecated legacy key. Kept only so loaders can warn and ignore it.
    pub(super) task_id_pattern: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawTaskSection {
    /// Removed pre-release selector. Kept here only so config loading can
    /// reject stale keys with an explicit task-artifacts cutover message.
    pub(super) artifact_store: Option<String>,
}
