use orbit_common::types::{AgentFamily, PlanningRoleAssignment, RoleSlot};

use super::super::{
    parse_planning_duel_signature, plan_artifact_for_assignment, planning_duel_plan_artifacts,
};
use super::{
    invalid_input_message, plan_artifact, planning_duel_artifacts, planning_roles, task_artifact,
    task_artifact_created_by,
};

#[test]
fn planning_duel_signature_extracts_family_and_slot() {
    let signature = parse_planning_duel_signature("*authored by: gemini / planner_a*\n## Plan")
        .expect("signature parses");
    assert_eq!(signature.family, AgentFamily::Gemini);
    assert_eq!(signature.slot, RoleSlot::PlannerA);

    assert!(parse_planning_duel_signature("*authored by: gemini*\n").is_err());
    assert!(parse_planning_duel_signature("*authored by: / planner_a*\n").is_err());
    assert!(parse_planning_duel_signature("*authored by: pro / planner_a*\n").is_err());
}

#[test]
fn planning_duel_plan_artifact_derives_planner_a_identity_from_metadata() {
    let artifacts = planning_duel_plan_artifacts(&[task_artifact_created_by(
        "planning-duel/planner_a.md",
        "## Plan\nGrok authored this plan without a signature.\n",
        "grok",
    )])
    .expect("metadata fallback should parse");

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].author.family, AgentFamily::Grok);
    assert_eq!(artifacts[0].slot, Some(RoleSlot::PlannerA));
}

#[test]
fn planning_duel_plan_artifacts_derives_both_planner_slots_from_path_and_metadata() {
    let artifacts = planning_duel_plan_artifacts(&[
        task_artifact_created_by(
            "planning-duel/planner_a.md",
            "## Plan\nPlanner A body.\n",
            "grok",
        ),
        task_artifact_created_by(
            "planning-duel/planner_b.md",
            "## Plan\nPlanner B body.\n",
            "codex",
        ),
    ])
    .expect("metadata fallback should parse both planners");

    assert_eq!(artifacts.len(), 2);
    assert_eq!(artifacts[0].path, "planning-duel/planner_a.md");
    assert_eq!(artifacts[0].author.family, AgentFamily::Grok);
    assert_eq!(artifacts[0].slot, Some(RoleSlot::PlannerA));
    assert_eq!(artifacts[1].path, "planning-duel/planner_b.md");
    assert_eq!(artifacts[1].author.family, AgentFamily::Codex);
    assert_eq!(artifacts[1].slot, Some(RoleSlot::PlannerB));
}

#[test]
fn planning_duel_plan_artifacts_preserves_current_and_legacy_signatures() {
    let artifacts = planning_duel_plan_artifacts(&[
        plan_artifact("planning-duel/planner_a.md", "gemini", "planner_a"),
        task_artifact(
            "planning-duel/planner_b.md",
            "*authored by: claude / claude-opus-4-7*\n## Plan\nLegacy shape.\n".to_string(),
        ),
    ])
    .expect("current and legacy signatures should parse");

    assert_eq!(artifacts[0].author.family, AgentFamily::Gemini);
    assert_eq!(artifacts[0].slot, Some(RoleSlot::PlannerA));
    assert_eq!(artifacts[1].author.family, AgentFamily::Claude);
    assert_eq!(artifacts[1].slot, Some(RoleSlot::PlannerB));
}

#[test]
fn planning_duel_plan_artifacts_rejects_malformed_authored_by_line_with_path() {
    let message = invalid_input_message(
        planning_duel_plan_artifacts(&[task_artifact_created_by(
            "planning-duel/planner_a.md",
            "*authored by: grok / *\n## Plan\nMalformed explicit signature.\n",
            "grok",
        )])
        .expect_err("malformed authored-by lines must not fall back to metadata"),
    );

    assert!(message.contains("planning-duel/planner_a.md"), "{message}");
    assert!(
        message.contains("signature must include both family and slot"),
        "{message}"
    );
}

#[test]
fn planning_duel_plan_artifacts_requires_usable_created_by_metadata_without_signature() {
    let missing_message = invalid_input_message(
        planning_duel_plan_artifacts(&[task_artifact(
            "planning-duel/planner_a.md",
            "## Plan\nNo signature and no metadata.\n".to_string(),
        )])
        .expect_err("missing created_by should fail"),
    );
    assert!(
        missing_message.contains("planning-duel/planner_a.md"),
        "{missing_message}"
    );
    assert!(
        missing_message.contains("missing trusted metadata field `created_by`"),
        "{missing_message}"
    );

    let unusable_message = invalid_input_message(
        planning_duel_plan_artifacts(&[task_artifact_created_by(
            "planning-duel/planner_b.md",
            "## Plan\nNo signature and unusable metadata.\n",
            "system",
        )])
        .expect_err("unusable created_by should fail"),
    );
    assert!(
        unusable_message.contains("planning-duel/planner_b.md"),
        "{unusable_message}"
    );
    assert!(
        unusable_message.contains("unusable trusted metadata field `created_by`"),
        "{unusable_message}"
    );
}

#[test]
fn plan_artifact_for_assignment_accepts_orb_00120_metadata_fallback_shape() {
    let artifacts = planning_duel_plan_artifacts(&[task_artifact_created_by(
        "planning-duel/planner_a.md",
        "## Plan\nGrok plan from ORB-00120 shape.\n",
        "grok",
    )])
    .expect("metadata fallback should parse ORB-00120 shape");
    let assignment = PlanningRoleAssignment {
        family: AgentFamily::Grok,
    };

    let artifact = plan_artifact_for_assignment(&artifacts, &assignment, RoleSlot::PlannerA)
        .expect("metadata-derived identity should match recorded assignment");

    assert_eq!(artifact.path, "planning-duel/planner_a.md");
}

#[test]
fn plan_artifact_validation_uses_family_and_slot_not_configured_model() {
    let artifacts = planning_duel_plan_artifacts(&[
        plan_artifact("planning-duel/planner_a.md", "gemini", "planner_a"),
        plan_artifact("planning-duel/planner_b.md", "codex", "planner_b"),
    ])
    .expect("plan artifacts parse");
    let assignment = PlanningRoleAssignment {
        family: AgentFamily::Gemini,
    };

    let artifact = plan_artifact_for_assignment(&artifacts, &assignment, RoleSlot::PlannerA)
        .expect("matching family and slot validate");

    assert_eq!(artifact.path, "planning-duel/planner_a.md");
}

#[test]
fn plan_artifact_validation_reports_family_mismatch() {
    let artifacts = planning_duel_plan_artifacts(&[plan_artifact(
        "planning-duel/planner_a.md",
        "claude",
        "planner_a",
    )])
    .expect("plan artifacts parse");
    let assignment = PlanningRoleAssignment {
        family: AgentFamily::Gemini,
    };

    let message = invalid_input_message(
        plan_artifact_for_assignment(&artifacts, &assignment, RoleSlot::PlannerA)
            .expect_err("mismatched family fails"),
    );

    assert!(message.contains("expected gemini"), "{message}");
    assert!(message.contains("has family claude"), "{message}");
}
