#![allow(missing_docs)]

use orbit_common::types::{
    AgentFamily, PlanningRoleAssignment, PlanningRoles, RoleSlot, TaskArtifact,
};
use serde_json::{Value, json};

use orbit_common::types::OrbitError;

fn task_artifact(path: &str, content: String) -> TaskArtifact {
    TaskArtifact::from_text(path, content)
}

fn task_artifact_created_by(path: &str, content: &str, created_by: &str) -> TaskArtifact {
    let mut artifact = TaskArtifact::from_text(path, content);
    artifact.created_by = Some(created_by.to_string());
    artifact
}

fn plan_artifact(path: &str, family: &str, slot: &str) -> TaskArtifact {
    task_artifact(
        path,
        format!("*authored by: {family} / {slot}*\n## Plan\nDo the thing.\n"),
    )
}

fn winner_marker(payload: Value) -> TaskArtifact {
    task_artifact(
        super::super::artifacts::WINNER_ARTIFACT_PATH,
        payload.to_string(),
    )
}

fn planning_roles() -> PlanningRoles {
    PlanningRoles {
        planner_a: PlanningRoleAssignment {
            family: AgentFamily::Codex,
        },
        planner_b: PlanningRoleAssignment {
            family: AgentFamily::Claude,
        },
        arbiter: PlanningRoleAssignment {
            family: AgentFamily::Gemini,
        },
    }
}

fn planning_duel_artifacts(winner_payload: Value) -> Vec<TaskArtifact> {
    vec![
        plan_artifact("planning-duel/planner_a.md", "codex", "planner_a"),
        plan_artifact("planning-duel/planner_b.md", "claude", "planner_b"),
        winner_marker(winner_payload),
    ]
}

fn invalid_input_message(error: OrbitError) -> String {
    match error {
        OrbitError::InvalidInput(message) => message,
        other => panic!("expected invalid input, got {other:?}"),
    }
}

// --- tests from plan.rs (uses to super:: helpers now local; super::super:: stay valid for planning_duel items) ---

// duplicate removed; types already imported above

use super::super::artifacts::{
    parse_planning_duel_signature, plan_artifact_for_assignment, planning_duel_plan_artifacts,
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

// --- tests from winner.rs (uses adjusted similarly; duplicates removed) ---

// duplicate json use removed

use super::super::artifacts::winner_artifact_from_artifacts;

#[test]
fn planning_duel_winner_marker_omits_derived_fields_when_roles_available() {
    let roles = planning_roles();
    let artifacts = planning_duel_artifacts(json!({
        "id": "T20260427-47",
        "winner_slot": "planner_b",
        "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
    }));

    let winner = winner_artifact_from_artifacts(&artifacts, Some(&roles))
        .expect("minimal winner marker should normalize");

    assert_eq!(winner.winner_family, AgentFamily::Claude);
    assert_eq!(winner.winner_slot, Some(RoleSlot::PlannerB));
    assert_eq!(winner.artifact_path, "planning-duel/planner_b.md");
    assert_eq!(winner.arbiter_family, AgentFamily::Gemini);
    assert_eq!(
        winner.arbiter_rationale,
        "Claude provided a more comprehensive diagnosis."
    );
}

#[test]
fn planning_duel_winner_marker_rejects_explicit_arbiter_mismatch() {
    let roles = planning_roles();
    let artifacts = planning_duel_artifacts(json!({
        "winner_slot": "planner_b",
        "arbiter_agent_cli": "codex",
        "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
    }));

    let message = invalid_input_message(
        winner_artifact_from_artifacts(&artifacts, Some(&roles))
            .expect_err("arbiter mismatch should be rejected"),
    );

    assert!(
        message.contains("winner artifact arbiter codex does not match recorded arbiter gemini"),
        "{message}"
    );
}

#[test]
fn planning_duel_winner_marker_requires_arbiter_identity_without_roles() {
    let artifacts = planning_duel_artifacts(json!({
        "winner_slot": "planner_b",
        "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
    }));

    let message = invalid_input_message(
        winner_artifact_from_artifacts(&artifacts, None)
            .expect_err("arbiter identity cannot be inferred without roles"),
    );

    assert!(
        message.contains(
            "planning duel winner marker requires `arbiter_family` when `planning_duel_roles` are unavailable"
        ),
        "{message}"
    );
}

#[test]
fn planning_duel_winner_marker_accepts_legacy_full_payload_without_roles() {
    let artifacts = planning_duel_artifacts(json!({
        "winner_agent_cli": "claude",
        "winner_model": "claude-opus-4-7",
        "artifact_path": "planning-duel/planner_b.md",
        "arbiter_agent_cli": "gemini",
        "arbiter_model": "gemini-3.1-pro",
        "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
    }));

    let winner = winner_artifact_from_artifacts(&artifacts, None)
        .expect("legacy full winner payload should still normalize");

    assert_eq!(winner.winner_family, AgentFamily::Claude);
    assert_eq!(winner.artifact_path, "planning-duel/planner_b.md");
    assert_eq!(winner.arbiter_family, AgentFamily::Gemini);
}
