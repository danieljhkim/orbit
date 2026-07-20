use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUsage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_create: u64,
    #[serde(default)]
    pub output: u64,
}

impl TokenUsage {
    pub fn prompt_response_total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolCallTrace {
    #[serde(default)]
    pub seq: u32,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub result_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_payload: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InvocationTrace {
    #[serde(default)]
    pub usage: TokenUsage,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTrace>,
    #[serde(default)]
    pub duration_ms: u64,
    /// Provider-reported total cost in USD for this invocation, when the
    /// caller has it (e.g. Claude CLI `total_cost_usd`). Kept verbatim
    /// alongside the token-derived cost computed from the price table at
    /// query time — it never overwrites the derived figure and is the
    /// ground truth for monthly manual reconciliation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cost_usd: Option<f64>,
}
