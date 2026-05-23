#![allow(missing_docs)]

//! Loop-block invariants for `loop_block.rs`: items-vs-`max_iterations`
//! validation, `break_when` exit, `LoopDidNotConverge` after exhaustion, and
//! body-failure exit. See task T20260509-7.

use super::*;

#[test]
fn loop_items_length_exceeding_max_iterations_errors() {
    let host = ScriptedHost::new([("noop", vec![])]);
    let job = job_with_steps(vec![loop_step(
        "spin",
        Some("{{ input.items }}"),
        2,
        None,
        vec![target_step("body", "noop")],
    )]);
    let writer = std::sync::Arc::new(test_writer("run-loop-too-many"));
    let err = execute_job(
        &job,
        json!({"items": [1, 2, 3]}),
        "run-loop-too-many",
        writer,
        &host,
    )
    .expect_err("too many items must error");
    assert!(matches!(err, DispatchError::JobExecution(_)));
}

#[test]
fn loop_break_when_exits_after_first_match_emits_broke_true() {
    // Iteration 1: body returns success but stop=false. Iteration 2: stop=true.
    // break_when fires; loop ends after 2 iterations. No `LoopDidNotConverge`.
    let host = ScriptedHost::new([(
        "step",
        vec![
            Action::Ok(json!({"stop": false})),
            Action::Ok(json!({"stop": true})),
        ],
    )]);
    let job = job_with_steps(vec![loop_step(
        "spin",
        None,
        5,
        Some("{{ steps.body.output.stop }} == true"),
        vec![target_step("body", "step")],
    )]);
    let writer = std::sync::Arc::new(test_writer("run-loop-break"));
    let outcome = execute_job(&job, Value::Null, "run-loop-break", writer.clone(), &host)
        .expect("execute_job ok");
    assert!(outcome.success);

    let events = writer.events_snapshot().expect("audit");
    let iter_ends: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            V2AuditEventKind::LoopIterationEnd {
                iteration, broke, ..
            } => Some((*iteration, *broke)),
            _ => None,
        })
        .collect();
    assert_eq!(iter_ends, vec![(1, false), (2, true)]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(&e.kind, V2AuditEventKind::LoopDidNotConverge { .. })),
        "did_not_converge must NOT emit on a successful break"
    );
    assert_eq!(host.call_count("step"), 2);
}

#[test]
fn loop_no_converge_after_max_iterations_emits_did_not_converge() {
    let host = ScriptedHost::new([("step", vec![])]);
    let job = job_with_steps(vec![loop_step(
        "spin",
        None,
        3,
        Some("a == b"), // never matches
        vec![target_step("body", "step")],
    )]);
    let writer = std::sync::Arc::new(test_writer("run-loop-no-converge"));
    let outcome = execute_job(
        &job,
        Value::Null,
        "run-loop-no-converge",
        writer.clone(),
        &host,
    )
    .expect("execute_job ok");
    assert!(outcome.success);
    assert_eq!(host.call_count("step"), 3);

    let events = writer.events_snapshot().expect("audit");
    let did_not_converge = events
        .iter()
        .find_map(|e| match &e.kind {
            V2AuditEventKind::LoopDidNotConverge { max_iterations, .. } => Some(*max_iterations),
            _ => None,
        })
        .expect("LoopDidNotConverge event");
    assert_eq!(did_not_converge, 3);
}

#[test]
fn loop_body_error_exits_loop_after_first_body_failure() {
    // Invariant: the first iteration whose body errors must terminate the
    // loop; subsequent iterations must not run. A retryable error with no
    // retry surfaces as Err from `run_step`, propagating out of `run_loop`.
    let host = ScriptedHost::new([(
        "step",
        vec![
            Action::Ok(json!({"i": 0})),
            Action::Err(DispatchError::DeterministicActionFailed {
                action: "step".into(),
                message: "broke".into(),
            }),
            Action::Ok(json!({"i": 2})),
        ],
    )]);
    let job = job_with_steps(vec![loop_step(
        "spin",
        None,
        5,
        Some("a == b"), // never matches
        vec![target_step("body", "step")],
    )]);
    let writer = std::sync::Arc::new(test_writer("run-loop-body-fail"));
    let err = execute_job(&job, Value::Null, "run-loop-body-fail", writer, &host)
        .expect_err("body error must surface as Err");

    assert!(matches!(
        err,
        DispatchError::DeterministicActionFailed { ref message, .. } if message == "broke"
    ));
    assert_eq!(
        host.call_count("step"),
        2,
        "loop must exit on first body failure (no third iteration)"
    );
}
