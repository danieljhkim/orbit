use serde_json::json;

use orbit_common::types::{AgentFamily, OrbitError, PlanningRoles, RoleSlot, TaskArtifact};

use super::super::select_planning_duel_roles;
use super::{TestHost, queue_permutation};

fn plan_artifact_local(path: &str, family: &str, slot: &str) -> TaskArtifact {
    TaskArtifact::from_text(
        path,
        format!("*authored by: {family} / {slot}*\n## Plan\nDo the thing.\n"),
    )
}

#[test]
fn planning_duel_e2e_select_roles_produces_assignment_that_validates_artifact() {
    // [2,0,1] places gemini in planner_a (matches the alias test above)
    for configured in ["pro", "gemini-3.1-pro"] {
        queue_permutation([2, 0, 1]);
        let host = TestHost::with_family_duel_model("gemini", configured);
        let output = select_planning_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
            .expect("select_planning_duel_roles with gemini duel model");

        let roles_value = output
            .get("planning_duel_roles")
            .expect("planning_duel_roles in output");
        let planning_roles: PlanningRoles =
            serde_json::from_value(roles_value.clone()).expect("parse PlanningRoles");
        let assignment = planning_roles.planner_a.clone();
        assert_eq!(assignment.family, AgentFamily::Gemini);

        // simulate artifact written with matching gemini signature for planner_a
        let raw_artifacts = vec![plan_artifact_local(
            "planning-duel/planner_a.md",
            "gemini",
            "planner_a",
        )];
        let plan_artifacts =
            super::super::super::artifacts::planning_duel_plan_artifacts(&raw_artifacts)
                .expect("planning_duel_plan_artifacts parses");
        let matched = super::super::super::artifacts::plan_artifact_for_assignment(
            &plan_artifacts,
            &assignment,
            RoleSlot::PlannerA,
        )
        .expect("plan_artifact_for_assignment succeeds when family+slot match");
        assert_eq!(matched.path, "planning-duel/planner_a.md");
    }

    // mismatch variant: gemini assigned but claude artifact present
    queue_permutation([2, 0, 1]);
    let host = TestHost::with_family_duel_model("gemini", "pro");
    let output =
        select_planning_duel_roles(&host, &json!({ "task_id": "ORB-TEST" })).expect("select");
    let roles_value = output.get("planning_duel_roles").expect("roles");
    let planning_roles: PlanningRoles = serde_json::from_value(roles_value.clone()).expect("parse");
    let assignment = planning_roles.planner_a.clone();

    let raw_artifacts = vec![plan_artifact_local(
        "planning-duel/planner_a.md",
        "claude",
        "planner_a",
    )];
    let plan_artifacts =
        super::super::super::artifacts::planning_duel_plan_artifacts(&raw_artifacts)
            .expect("parse");
    let err = super::super::super::artifacts::plan_artifact_for_assignment(
        &plan_artifacts,
        &assignment,
        RoleSlot::PlannerA,
    )
    .expect_err("mismatch must fail");
    let msg = match err {
        OrbitError::InvalidInput(m) => m,
        other => panic!("expected InvalidInput, got {other:?}"),
    };
    assert!(msg.contains("expected gemini"), "msg={msg}");
    assert!(msg.contains("has family claude"), "msg={msg}");
}

#[test]
fn planning_duel_roles_explicit_assignment_returns_requested_families_without_permutation_queue() {
    // Explicit families provided in input; must NOT require seeding TEST_PERMUTATION_QUEUE
    // and must bypass next_permutation entirely.
    let host = TestHost::new();
    let input = json!({
        "task_id": "ORB-TEST-EXPLICIT",
        "planner_a_family": "gemini",
        "planner_b_family": "codex",
        "arbiter_family": "grok"
    });

    let output = select_planning_duel_roles(&host, &input)
        .expect("explicit assignment path succeeds without queue");

    assert_eq!(output["planner_a_agent_cli"], "gemini");
    assert_eq!(output["planner_b_agent_cli"], "codex");
    assert_eq!(output["arbiter_agent_cli"], "grok");
    assert_eq!(
        output["planning_duel_roles"]["planner_a"]["family"],
        "gemini"
    );
    assert_eq!(
        output["planning_duel_roles"]["planner_b"]["family"],
        "codex"
    );
    assert_eq!(output["planning_duel_roles"]["arbiter"]["family"], "grok");
}

#[test]
fn planning_duel_roles_no_override_still_uses_queued_permutation_path() {
    // No override fields -> must still go through next_permutation (test seeds queue)
    queue_permutation([0, 2, 3]); // codex, gemini, grok
    let host = TestHost::new();
    let input = json!({ "task_id": "ORB-TEST-NO-OVERRIDE" });

    let output =
        select_planning_duel_roles(&host, &input).expect("no-override still uses permutation");

    assert_eq!(output["planner_a_agent_cli"], "codex");
    assert_eq!(output["planner_b_agent_cli"], "gemini");
    assert_eq!(output["arbiter_agent_cli"], "grok");
}
