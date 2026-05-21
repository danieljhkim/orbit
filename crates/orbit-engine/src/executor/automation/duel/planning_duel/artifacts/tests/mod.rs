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
    task_artifact(super::WINNER_ARTIFACT_PATH, payload.to_string())
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

mod plan;
mod winner;
