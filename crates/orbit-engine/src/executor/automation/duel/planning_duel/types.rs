use orbit_common::types::EfficiencyMetrics;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PlanningDuelRoleMetrics {
    pub agent: String,
    pub model: String,
    pub activity_id: String,
    pub efficiency: PlanningDuelEfficiency,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelEfficiency {
    pub invocation_count: u64,
    pub wall_clock_ms: u64,
    pub tool_call_count: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_create_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub byte_proxy_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelWinnerArtifact {
    pub winner_agent_cli: String,
    pub winner_model: String,
    pub artifact_path: String,
    pub arbiter_agent_cli: String,
    pub arbiter_model: String,
    pub arbiter_rationale: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelWinnerMarker {
    pub winner_agent_cli: String,
    pub winner_model: String,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub arbiter_agent_cli: Option<String>,
    #[serde(default)]
    pub arbiter_model: Option<String>,
    pub arbiter_rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningDuelPlanArtifact {
    pub path: String,
    pub content: String,
    pub author: orbit_common::types::PlanningRoleAssignment,
}

pub(super) fn into_efficiency_metrics(value: PlanningDuelEfficiency) -> EfficiencyMetrics {
    let token_usage = orbit_common::types::TokenUsage {
        input: value.input_tokens,
        cache_read: value.cache_read_tokens,
        cache_create: value.cache_create_tokens,
        output: value.output_tokens,
    };
    let has_exact_tokens = token_usage.input > 0
        || token_usage.cache_read > 0
        || token_usage.cache_create > 0
        || token_usage.output > 0;

    EfficiencyMetrics {
        wall_clock_ms: value.wall_clock_ms,
        tool_call_count: value.tool_call_count.min(u32::MAX as u64) as u32,
        token_usage: has_exact_tokens.then_some(token_usage),
        byte_proxy_total: (!has_exact_tokens && value.byte_proxy_total > 0)
            .then_some(value.byte_proxy_total),
    }
}
