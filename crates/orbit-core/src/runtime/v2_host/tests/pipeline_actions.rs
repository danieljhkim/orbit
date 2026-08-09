use super::super::pipeline_actions::*;
use crate::OrbitRuntime;
use orbit_engine::DispatchError;
use serde_json::json;

fn action_failure_message(err: DispatchError, expected_action: &str) -> String {
    match err {
        DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, expected_action);
            message
        }
        other => panic!("expected deterministic action failure, got {other}"),
    }
}

#[test]
fn pipeline_success_guard_accepts_succeeded_result() {
    let output = pipeline_success_guard(
        "pipeline_success_guard",
        &json!({
            "result": {
                "run_id": "jrun-ok",
                "status": "succeeded"
            }
        }),
    )
    .expect("succeeded result should pass");

    assert_eq!(output["succeeded"], json!(true));
    assert_eq!(output["checked_count"], json!(1));
}

#[test]
fn pipeline_success_guard_rejects_failed_result() {
    let err = pipeline_success_guard(
        "pipeline_success_guard",
        &json!({
            "context": "task gate child",
            "result": {
                "run_id": "jrun-failed",
                "status": "failed",
                "error": "implementation failed"
            }
        }),
    )
    .expect_err("failed child run should fail the guard");

    let message = action_failure_message(err, "pipeline_success_guard");
    assert!(message.contains("task gate child did not succeed"));
    assert!(message.contains("jrun-failed"));
    assert!(message.contains("status failed"));
    assert!(message.contains("implementation failed"));
}

#[test]
fn pipeline_success_guard_rejects_mixed_results() {
    let err = pipeline_success_guard(
        "pipeline_success_guard",
        &json!({
            "results": [
                {
                    "run_id": "jrun-ok",
                    "status": "succeeded"
                },
                {
                    "run_id": "jrun-cancelled",
                    "status": "cancelled"
                },
                null
            ]
        }),
    )
    .expect_err("any non-succeeded result should fail the guard");

    let message = action_failure_message(err, "pipeline_success_guard");
    assert!(message.contains("results[1] run jrun-cancelled status cancelled"));
    assert!(message.contains("results[2] missing string status"));
}

#[test]
fn gate_starvation_fail_names_both_conflicting_files_and_unmet_dependencies() {
    // A dependency-starved gate previously reported an empty
    // `conflicting_files` list and named no blocker at all.
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let message = action_failure_message(
        gate_starvation_fail(
            &runtime,
            "gate_starvation_fail",
            &json!({
                "task_ids": ["ORB-2"],
                "conflicts": [],
                "waiting_on_deps": ["ORB-1"],
                "max_wait_seconds": 3600,
            }),
        )
        .expect_err("starvation always fails the run"),
        "gate_starvation_fail",
    );

    assert!(message.contains("gate.starvation"), "{message}");
    assert!(message.contains("ORB-1"), "{message}");
    assert!(message.contains("waiting_on_deps"), "{message}");
}

#[test]
fn gate_starvation_fail_tolerates_a_missing_waiting_on_deps_input() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");

    let message = action_failure_message(
        gate_starvation_fail(
            &runtime,
            "gate_starvation_fail",
            &json!({
                "task_ids": ["ORB-2"],
                "conflicts": [{ "file": "file:src/lib.rs", "held_by": "task", "held_by_id": "ORB-3" }],
            }),
        )
        .expect_err("starvation always fails the run"),
        "gate_starvation_fail",
    );

    assert!(message.contains("file:src/lib.rs"), "{message}");
    assert!(message.contains("waiting_on_deps=[]"), "{message}");
}
