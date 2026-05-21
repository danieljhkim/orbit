use serde_json::json;

use super::super::{build_roles_output, select_planning_duel_roles}; // widened to pub(crate) for test surface
use super::{TestHost, queue_permutation};

#[test]
fn planning_duel_role_output_can_assign_grok() {
    let host = TestHost::new();
    let output = build_roles_output(&host, [3, 0, 1]).expect("roles output");

    assert_eq!(output["planner_a_agent_cli"], "grok");
    assert_eq!(output["planner_a_model"], "grok-4");
    assert_eq!(output["planning_duel_roles"]["planner_a"]["family"], "grok");
    assert!(output["planning_duel_roles"]["planner_a"]["model"].is_null());
    assert_eq!(output["planner_b_agent_cli"], "codex");
    assert_eq!(output["arbiter_agent_cli"], "claude");
}

#[test]
fn planning_duel_roles_prefer_duel_model_then_resolved_pair() {
    queue_permutation([0, 1, 2]);
    let host = TestHost::with_duel_model(Some("M_duel"));
    let output = select_planning_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
        .expect("planning role selection uses duel model");
    assert_eq!(output["planner_a_model"], "M_duel");
    assert_eq!(
        output["planning_duel_roles"]["planner_a"]["family"],
        "codex"
    );

    queue_permutation([0, 1, 2]);
    let host = TestHost::with_duel_model(None);
    let output = select_planning_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
        .expect("planning role selection falls back to resolved pair");
    assert_eq!(output["planner_a_model"], "M_exec");
}

#[test]
fn planning_duel_role_selection_keeps_family_identity_for_model_aliases() {
    for model in ["pro", "gemini-3.1-pro"] {
        queue_permutation([2, 0, 1]);
        let host = TestHost::with_family_duel_model("gemini", model);
        let output = select_planning_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
            .expect("planning role selection uses configured gemini model");

        assert_eq!(output["planner_a_agent_cli"], "gemini");
        assert_eq!(output["planner_a_model"], model);
        assert_eq!(
            output["planning_duel_roles"]["planner_a"]["family"],
            "gemini"
        );
        assert!(output["planning_duel_roles"]["planner_a"]["model"].is_null());
    }
}
