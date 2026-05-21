use orbit_common::types::{AgentFamily, RoleSlot};
use serde_json::json;

use super::super::winner_artifact_from_artifacts;
use super::{invalid_input_message, planning_duel_artifacts, planning_roles};

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
