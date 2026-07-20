use orbit_common::types::{
    AgentFamily, EfficiencyMetrics, PlanningRoleAssignment, RoleSlot, TokenUsage,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PlanningDuelRoleMetrics {
    pub family: AgentFamily,
    pub slot: RoleSlot,
    pub activity_id: String,
    pub efficiency: PlanningDuelEfficiency,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelEfficiency {
    pub invocation_count: u64,
    pub wall_clock_ms: u64,
    pub tool_call_count: u64,
    pub token_usage: Option<TokenUsage>,
    pub byte_proxy_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelWinnerArtifact {
    pub winner_family: AgentFamily,
    pub winner_slot: Option<RoleSlot>,
    pub artifact_path: String,
    pub arbiter_family: AgentFamily,
    pub arbiter_rationale: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(super) struct PlanningDuelWinnerMarker {
    #[serde(default)]
    pub winner_slot: Option<RoleSlot>,
    #[serde(default)]
    pub winner_agent_cli: String,
    #[serde(default)]
    pub winner_model: String,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub arbiter_agent_cli: Option<String>,
    #[serde(default)]
    pub arbiter_model: Option<String>,
    #[serde(default)]
    pub arbiter_family: Option<AgentFamily>,
    pub arbiter_rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningDuelPlanArtifact {
    pub path: String,
    pub content: String,
    pub author: PlanningRoleAssignment,
    pub slot: Option<RoleSlot>,
}

pub(super) fn into_efficiency_metrics(value: PlanningDuelEfficiency) -> EfficiencyMetrics {
    let has_token_usage = value.token_usage.is_some();

    EfficiencyMetrics {
        wall_clock_ms: value.wall_clock_ms,
        tool_call_count: value.tool_call_count.min(u32::MAX as u64) as u32,
        token_usage: value.token_usage,
        byte_proxy_total: (!has_token_usage && value.byte_proxy_total > 0)
            .then_some(value.byte_proxy_total),
    }
}
