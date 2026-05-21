use serde_json::json;

use orbit_common::types::TaskStatus;

use crate::context::TaskReadHost;

use super::super::run_planning_duel;
use super::PlanningDuelHost;

#[test]
fn run_planning_duel_preserves_existing_task_status() {
    for status in [
        TaskStatus::Proposed,
        TaskStatus::Friction,
        TaskStatus::Backlog,
        TaskStatus::Rejected,
        TaskStatus::Archived,
        TaskStatus::InProgress,
    ] {
        let host = PlanningDuelHost::new(status);
        let output = run_planning_duel(
            &host,
            &json!({
                "task_id": "T20260430-STATUS",
                "run_id": format!("jrun-{status}")
            }),
            false,
        )
        .expect("planning duel succeeds without lifecycle admission");

        let expected_status = status.to_string();
        let comments = host
            .get_task_comments("T20260430-STATUS")
            .expect("comments remain readable");
        let comment = comments.last().expect("planning duel comment");
        assert_eq!(host.task_status(), status, "{status}");
        assert_eq!(
            output["task_status"].as_str(),
            Some(expected_status.as_str()),
            "{status}"
        );
        assert!(
            comment
                .message
                .contains(&format!("Task status remains {expected_status}.")),
            "{status}: {}",
            comment.message
        );
        assert!(
            !comment
                .message
                .contains("Task status is in-progress for workflow execution."),
            "{status}: {}",
            comment.message
        );
        assert_eq!(host.admission_count(), 0, "{status}");
        assert_eq!(host.start_count(), 0, "{status}");
    }
}

#[test]
fn missing_planner_artifact_error_includes_child_invocation_diagnostics() {
    let host = PlanningDuelHost::new(TaskStatus::InProgress);
    host.omit_planner_artifacts();

    let err = run_planning_duel(
        &host,
        &json!({
            "task_id": "T20260430-STATUS",
            "run_id": "jrun-missing-planner-artifact"
        }),
        false,
    )
    .expect_err("missing planner artifact should fail");
    let message = err.to_string();

    assert!(
        message.contains("missing planning duel artifact for"),
        "{message}"
    );
    assert!(
        message.contains("stderr_blob_ref=stderr-digest"),
        "{message}"
    );
    assert!(
        message.contains("store_error: attempt to write a readonly database"),
        "{message}"
    );
    assert!(
        message.contains("tool_calls=orbit.duel.plan.add"),
        "{message}"
    );
}
