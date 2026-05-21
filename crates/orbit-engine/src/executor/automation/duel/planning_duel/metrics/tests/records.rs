use chrono::Utc;
use orbit_common::types::{AgentFamily, PlanningRoleAssignment, RoleSlot};
use orbit_store::InvocationRecord;

use super::super::matching_role_records;

#[test]
fn metrics_attribute_invocations_by_family_and_slot_not_model() {
    let role = PlanningRoleAssignment {
        family: AgentFamily::Gemini,
    };
    let records = vec![
        invocation_record(
            "propose_duel_plan",
            "gemini",
            Some("gemini-3.1-pro"),
            Some(RoleSlot::PlannerA),
        ),
        invocation_record(
            "propose_duel_plan",
            "gemini",
            Some("pro"),
            Some(RoleSlot::PlannerB),
        ),
        invocation_record(
            "propose_duel_plan",
            "claude",
            Some("claude-opus-4-7"),
            Some(RoleSlot::PlannerA),
        ),
    ];

    let matching = matching_role_records(&records, &role, RoleSlot::PlannerA);

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].model.as_deref(), Some("gemini-3.1-pro"));
    assert_eq!(matching[0].slot, Some(RoleSlot::PlannerA));
}

fn invocation_record(
    activity_id: &str,
    agent: &str,
    model: Option<&str>,
    slot: Option<RoleSlot>,
) -> InvocationRecord {
    InvocationRecord {
        id: 1,
        ts: Utc::now(),
        job_run_id: "jrun-1".to_string(),
        activity_id: activity_id.to_string(),
        agent: agent.to_string(),
        model: model.map(ToOwned::to_owned),
        slot,
        duration_ms: 100,
        input_tokens: 1,
        cache_read_tokens: 0,
        cache_create_tokens: 0,
        output_tokens: 1,
        total_tokens: 2,
        tool_call_count: 1,
        task_ids: Vec::new(),
        tool_calls: Vec::new(),
    }
}
