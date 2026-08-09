use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawRuntimeConfig {
    // Deliberately no `deny_unknown_fields`: config.toml is also home to
    // independently parsed surfaces such as `[docs]`. Runtime admission reads
    // only its owned keys, while explicit migration guards below reject retired
    // runtime keys whose continued acceptance would be unsafe or misleading.
    #[allow(dead_code)]
    pub(super) identity: Option<toml::Value>,
    pub(super) task: Option<RawTaskSection>,
    pub(super) knowledge: Option<RawKnowledgeConfig>,
    pub(super) watch: Option<toml::Value>,
    /// Removed in ORB-00058. Kept only so config loading can reject stale
    /// `[agent.<role>]` tables with an explicit migration error.
    pub(super) agent: Option<BTreeMap<String, toml::Value>>,
    /// `[crews.<name>]` registry. Each table supplies one assignment Orbit
    /// resolves for activity dispatch at run start.
    pub(super) crews: Option<BTreeMap<String, RawCrewEntry>>,
    /// Retired in ORB-10627. Existing workspaces may still carry the section
    /// written by older `orbit init`; loaders warn and ignore it.
    pub(super) duel: Option<toml::Value>,
}

/// Schema for one provider-model-backend crew assignment.
///
/// Serialize is derived so the writer in `bootstrap` can emit fresh entries
/// without hand-rolling TOML. The struct is `pub` so the CLI can hand a map
/// of these directly into `InitOptions::crew_settings` when running
/// interactive prompts.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RawCrewAssignment {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Retired role tables are retained only to reject stale configuration
    /// with rewrite guidance at load time. They are not part of the crew
    /// schema and never participate in assignment resolution or serialization.
    #[serde(default, skip_serializing)]
    pub planner: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing)]
    pub implementer: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing)]
    pub reviewer: Option<BTreeMap<String, String>>,
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
