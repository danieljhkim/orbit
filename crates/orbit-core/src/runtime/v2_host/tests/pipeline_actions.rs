use super::super::pipeline_actions::*;
use crate::OrbitRuntime;
use orbit_engine::DispatchError;
use serde_json::json;

fn action_failure_message(err: DispatchError) -> String {
    match err {
        DispatchError::DeterministicActionFailed { action, message } => {
            assert_eq!(action, "pipeline_success_guard");
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

    let message = action_failure_message(err);
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

    let message = action_failure_message(err);
    assert!(message.contains("results[1] run jrun-cancelled status cancelled"));
    assert!(message.contains("results[2] missing string status"));
}

#[test]
fn invoke_and_wait_dedupe_reuses_one_run_and_rejects_multiple_matches() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let first = runtime
        .stores()
        .jobs()
        .insert_job_run(
            "task_review_pipeline",
            1,
            chrono::Utc::now(),
            Some(json!({ "parent_run_id": "jrun-parent" })),
            None,
        )
        .expect("insert first review run");
    let action_input = json!({ "dedupe_run_input_field": "parent_run_id" });
    let run_input = json!({ "parent_run_id": "jrun-parent" });

    let reused = deduped_child_run_id(
        &runtime,
        "invoke_and_wait",
        &action_input,
        "task_review_pipeline",
        &run_input,
    )
    .expect("dedupe lookup");
    assert_eq!(reused.as_deref(), Some(first.run_id.as_str()));

    runtime
        .stores()
        .jobs()
        .insert_job_run(
            "task_review_pipeline",
            1,
            chrono::Utc::now(),
            Some(json!({ "parent_run_id": "jrun-parent" })),
            None,
        )
        .expect("insert duplicate review run");
    let error = deduped_child_run_id(
        &runtime,
        "invoke_and_wait",
        &action_input,
        "task_review_pipeline",
        &run_input,
    )
    .expect_err("multiple matching review runs fail closed");
    assert!(matches!(
        error,
        DispatchError::DeterministicActionFailed { .. }
    ));
}

#[test]
fn independent_review_guard_accepts_both_exact_head_verdicts() {
    for verdict in ["approve", "request_changes"] {
        let output = independent_review_guard(
            "independent_review_guard",
            &json!({
                "verdict": verdict,
                "reviewed_head_sha": "abc123",
                "candidate_head_sha": "abc123",
            }),
        )
        .expect("exact-head verdict passes");

        assert_eq!(output["verdict"], verdict);
        assert_eq!(output["reviewed_head_sha"], "abc123");
        assert_eq!(output["exact_head"], true);
    }
}

#[test]
fn independent_review_guard_fails_closed_on_missing_or_mismatched_verdict() {
    for input in [
        json!({
            "reviewed_head_sha": "abc123",
            "candidate_head_sha": "abc123",
        }),
        json!({
            "verdict": "looks_good",
            "reviewed_head_sha": "abc123",
            "candidate_head_sha": "abc123",
        }),
        json!({
            "verdict": "approve",
            "reviewed_head_sha": "def456",
            "candidate_head_sha": "abc123",
        }),
    ] {
        let error = independent_review_guard("independent_review_guard", &input)
            .expect_err("invalid verdict must fail");
        assert!(matches!(
            error,
            DispatchError::DeterministicActionFailed { .. }
        ));
    }
}
