use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutorDef {
    pub name: String,
    /// Executor family, such as "agent_cli", "direct_agent", or "cli_command".
    pub executor_type: String,
    /// For agent_cli: the CLI command (e.g., "claude", "codex")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Expected stdout format: "envelope", "json", "text"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_format: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ExecutorDef {
    pub fn model_for_tier(&self, tier: &str) -> Option<&str> {
        self.models
            .get(tier)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}
