use super::super::pipeline_actions::*;
use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_common::types::TaskStatus;
use orbit_engine::DispatchError;
use serde_json::{Value, json};

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

const CANDIDATE_SHA: &str = "abc123";

fn reviewed_task(runtime: &OrbitRuntime, criteria: &[&str]) -> String {
    runtime
        .add_task(TaskAddParams {
            title: "reviewed candidate".to_string(),
            description: "published for independent review".to_string(),
            acceptance_criteria: criteria.iter().map(|value| value.to_string()).collect(),
            status: Some(TaskStatus::InProgress),
            ..TaskAddParams::default()
        })
        .expect("add reviewed task")
        .id
}

fn append_comment(runtime: &OrbitRuntime, task_id: &str, message: &str) {
    runtime
        .update_task(
            task_id,
            TaskUpdateParams {
                comment: Some(message.to_string()),
                ..TaskUpdateParams::default()
            },
        )
        .expect("append task comment");
}

/// Persist the reviewer's durable record exactly as `agent_review` is told to.
fn persist_review_record(
    runtime: &OrbitRuntime,
    task_id: &str,
    verdict: &str,
    reconciled_through: chrono::DateTime<chrono::Utc>,
    criteria: Value,
) {
    let payload = json!({
        "candidate_head_sha": CANDIDATE_SHA,
        "verdict": verdict,
        "reconciled_through": reconciled_through.to_rfc3339(),
        "late_corrections": [],
        "criteria": criteria,
    });
    append_comment(
        runtime,
        task_id,
        &format!("{REVIEW_RECORD_MARKER}\n{payload}"),
    );
}

fn guard_input(task_id: &str, verdict: &str) -> Value {
    json!({
        "verdict": verdict,
        "reviewed_head_sha": CANDIDATE_SHA,
        "candidate_head_sha": CANDIDATE_SHA,
        "task_ids": [task_id],
    })
}

fn latest_authority(runtime: &OrbitRuntime, task_id: &str) -> chrono::DateTime<chrono::Utc> {
    runtime
        .get_task_comments(task_id)
        .expect("read comments")
        .iter()
        .map(|comment| comment.at)
        .max()
        .unwrap_or_else(chrono::Utc::now)
}

#[test]
fn independent_review_guard_accepts_a_fully_covered_reconciled_approval() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one", "criterion two"]);
    append_comment(&runtime, &task_id, "scope note from the task owner");
    persist_review_record(
        &runtime,
        &task_id,
        "approve",
        latest_authority(&runtime, &task_id),
        json!([
            {"index": 1, "verdict": "met", "evidence": "src/lib.rs:10"},
            {"index": 2, "verdict": "met", "evidence": "src/lib.rs:42"},
        ]),
    );

    let output = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect("covered, reconciled approval passes");

    assert_eq!(output["verdict"], "approve");
    assert_eq!(output["reviewed_head_sha"], CANDIDATE_SHA);
    assert_eq!(output["exact_head"], true);
    assert_eq!(output["criteria_covered"], json!(2));
}

#[test]
fn independent_review_guard_accepts_request_changes_without_full_coverage() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one", "criterion two"]);
    persist_review_record(
        &runtime,
        &task_id,
        "request_changes",
        latest_authority(&runtime, &task_id),
        json!([{"index": 1, "verdict": "not_met", "evidence": "scope violation"}]),
    );

    let output = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "request_changes"),
    )
    .expect("a blocking verdict is recorded, not coverage-gated");

    assert_eq!(output["verdict"], "request_changes");
}

/// The PR #638 shape: the candidate implemented behavior a later task comment
/// explicitly deferred, and the reviewer approved against the pre-correction
/// contract. The approval is reconciled through an older point than the
/// correction, so the guard refuses it.
#[test]
fn independent_review_guard_rejects_approval_that_ignores_a_later_correction() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one"]);
    append_comment(&runtime, &task_id, "original scope");
    let reconciled_through = latest_authority(&runtime, &task_id);
    append_comment(
        &runtime,
        &task_id,
        "scope correction: capability gating is deferred to a follow-up",
    );
    persist_review_record(
        &runtime,
        &task_id,
        "approve",
        reconciled_through,
        json!([{"index": 1, "verdict": "met", "evidence": "src/lib.rs:10"}]),
    );

    let error = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect_err("approval that predates the correction must fail");

    let message = action_failure_message(error, "independent_review_guard");
    assert!(message.contains("later task comment"), "{message}");
    assert!(message.contains(&task_id), "{message}");
}

#[test]
fn independent_review_guard_rejects_approval_missing_a_criterion_verdict() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one", "criterion two"]);
    persist_review_record(
        &runtime,
        &task_id,
        "approve",
        latest_authority(&runtime, &task_id),
        json!([{"index": 1, "verdict": "met", "evidence": "src/lib.rs:10"}]),
    );

    let error = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect_err("partial criterion coverage must fail");

    let message = action_failure_message(error, "independent_review_guard");
    assert!(
        message.contains("no verdict for acceptance criteria"),
        "{message}"
    );
}

#[test]
fn independent_review_guard_rejects_approval_reporting_an_unmet_criterion() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one"]);
    persist_review_record(
        &runtime,
        &task_id,
        "approve",
        latest_authority(&runtime, &task_id),
        json!([{"index": 1, "verdict": "not_met", "evidence": "still missing"}]),
    );

    let error = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect_err("approving an unmet criterion must fail");

    let message = action_failure_message(error, "independent_review_guard");
    assert!(message.contains("not met"), "{message}");
}

#[test]
fn independent_review_guard_requires_a_persisted_record_for_the_candidate() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one"]);
    append_comment(
        &runtime,
        &task_id,
        &format!(
            "{REVIEW_RECORD_MARKER}\n{}",
            json!({
                "candidate_head_sha": "stale999",
                "verdict": "approve",
                "reconciled_through": chrono::Utc::now().to_rfc3339(),
                "late_corrections": [],
                "criteria": [{"index": 1, "verdict": "met"}],
            })
        ),
    );

    let error = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect_err("a record for another candidate does not count");

    let message = action_failure_message(error, "independent_review_guard");
    assert!(message.contains("persisted no"), "{message}");
    assert!(message.contains(CANDIDATE_SHA), "{message}");
}

#[test]
fn independent_review_guard_rejects_a_response_that_contradicts_the_records() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one"]);
    persist_review_record(
        &runtime,
        &task_id,
        "request_changes",
        latest_authority(&runtime, &task_id),
        json!([{"index": 1, "verdict": "not_met", "evidence": "regression"}]),
    );

    let error = independent_review_guard(
        &runtime,
        "independent_review_guard",
        &guard_input(&task_id, "approve"),
    )
    .expect_err("an approving response over a blocking record must fail");

    let message = action_failure_message(error, "independent_review_guard");
    assert!(message.contains("verdict mismatch"), "{message}");
}

#[test]
fn independent_review_guard_fails_closed_on_missing_bundle_or_mismatched_head() {
    let runtime = OrbitRuntime::in_memory().expect("runtime");
    let task_id = reviewed_task(&runtime, &["criterion one"]);
    persist_review_record(
        &runtime,
        &task_id,
        "approve",
        latest_authority(&runtime, &task_id),
        json!([{"index": 1, "verdict": "met"}]),
    );

    for input in [
        json!({
            "verdict": "looks_good",
            "reviewed_head_sha": CANDIDATE_SHA,
            "candidate_head_sha": CANDIDATE_SHA,
            "task_ids": [task_id.clone()],
        }),
        json!({
            "verdict": "approve",
            "reviewed_head_sha": "def456",
            "candidate_head_sha": CANDIDATE_SHA,
            "task_ids": [task_id.clone()],
        }),
        json!({
            "verdict": "approve",
            "reviewed_head_sha": CANDIDATE_SHA,
            "candidate_head_sha": CANDIDATE_SHA,
        }),
    ] {
        let error = independent_review_guard(&runtime, "independent_review_guard", &input)
            .expect_err("invalid guard input must fail");
        assert!(matches!(
            error,
            DispatchError::DeterministicActionFailed { .. }
        ));
    }
}
