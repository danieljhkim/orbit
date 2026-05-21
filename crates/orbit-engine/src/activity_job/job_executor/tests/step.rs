#![allow(missing_docs)]

//! Step retry, short-circuit, and backoff invariants for `step.rs`.
//! Each test names the specific invariant or failure mode it guards.
//! See task T20260509-7.

use super::*;

#[test]
fn linear_step_success_propagates_output_to_pipeline() {
    // Invariant: a successful step's value lands in `pipeline[step.id]` so
    // downstream steps can consume it via `{{ steps.<id>.output.* }}`.
    let host = ScriptedHost::new([("build", vec![Action::Ok(json!({"ok": true}))])]);
    let job = job_with_steps(vec![target_step("build", "build")]);

    let outcome = run_job(&host, &job, Value::Null, "run-linear-success");

    assert!(outcome.success);
    let pipeline = outcome.pipeline.as_object().expect("pipeline is an object");
    assert_eq!(pipeline.get("build"), Some(&json!({"ok": true})));
}

#[test]
fn when_false_literal_skips_step_without_failing_job() {
    let host = ScriptedHost::new([("disabled", vec![Action::Ok(json!({"ran": true}))])]);
    let mut skipped = target_step("safety", "disabled");
    skipped.when = Some("false".to_string());
    let job = job_with_steps(vec![skipped]);
    let writer = std::sync::Arc::new(test_writer("run-when-false-literal"));

    let outcome = execute_job(
        &job,
        Value::Null,
        "run-when-false-literal",
        writer.clone(),
        &host,
    )
    .expect("when:false should skip cleanly");

    assert!(outcome.success);
    assert_eq!(host.call_count("disabled"), 0, "skipped step must not run");
    let events = writer.events_snapshot().expect("audit");
    assert!(
        events.iter().any(|event| matches!(
            &event.kind,
            V2AuditEventKind::StepSkipped { step_id, reason }
                if step_id == "safety" && reason == "when:false => false"
        )),
        "expected StepSkipped audit event for when:false"
    );
}

#[test]
fn step_failure_short_circuits_remaining_steps() {
    // Invariant: a failed step terminates the linear loop in `execute_job`
    // (mod.rs:131-148). Without retry/recovery, a retryable
    // DeterministicActionFailed bubbles up as Err — and crucially, later
    // steps must not have been invoked.
    let host = ScriptedHost::new([
        (
            "first",
            vec![Action::Err(DispatchError::DeterministicActionFailed {
                action: "first".into(),
                message: "boom".into(),
            })],
        ),
        ("second", vec![Action::Ok(json!({"ran": true}))]),
    ]);
    let job = job_with_steps(vec![
        target_step("step1", "first"),
        target_step("step2", "second"),
    ]);
    let writer = std::sync::Arc::new(test_writer("run-shortcircuit"));

    let err = execute_job(&job, Value::Null, "run-shortcircuit", writer.clone(), &host)
        .expect_err("first step error must surface");

    assert!(matches!(
        err,
        DispatchError::DeterministicActionFailed { ref action, .. } if action == "first"
    ));
    assert_eq!(host.call_count("second"), 0, "second step must not run");
    let events = writer.events_snapshot().expect("audit");
    let step_finished = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::StepFinished {
                step_id,
                outcome,
                error_message,
            } if step_id == "step1" => Some((outcome, error_message)),
            _ => None,
        })
        .expect("step finished event");
    assert_eq!(step_finished.0, "error");
    assert_eq!(
        step_finished.1.as_deref(),
        Some("deterministic action `first` failed: boom")
    );
}

#[cfg(unix)]
#[test]
fn non_success_step_outcome_copies_step_message_to_finished_audit_event() {
    let host = ScriptedHost::new([("unused", Vec::new())]);
    let step = JobV2Step {
        id: "shell_fail".to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Target(TargetStep {
            spec: ActivityV2Spec::Shell(orbit_common::types::activity_job::ShellSpec {
                program: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "exit 7".to_string()],
                allowed_programs: vec!["/bin/sh".to_string()],
                timeout_seconds: 0,
                expected_exit_codes: vec![0],
            }),
            activity_name: None,
            fs_profile: None,
            default_input: None,
            timeout_seconds: 0,
            session: None,
            role: None,
        }),
    };
    let job = job_with_steps(vec![step]);
    let writer = std::sync::Arc::new(test_writer("run-shell-fail"));

    let outcome =
        execute_job(&job, Value::Null, "run-shell-fail", writer.clone(), &host).expect("run job");

    assert!(!outcome.success);
    assert_eq!(outcome.message.as_deref(), Some("exit 7 not in [0]"));
    let events = writer.events_snapshot().expect("audit");
    let step_finished = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::StepFinished {
                step_id,
                outcome,
                error_message,
            } if step_id == "shell_fail" => Some((outcome, error_message)),
            _ => None,
        })
        .expect("step finished event");
    assert_eq!(step_finished.0, "failed");
    assert_eq!(step_finished.1.as_deref(), Some("exit 7 not in [0]"));
}

#[test]
fn retry_runs_max_attempts_then_surfaces_last_error() {
    // Invariant: with retry, a deterministic action that always errors is
    // retried up to `max_attempts`; the final error surfaces as Err and
    // `StepRetry` is emitted between attempts (N attempts → N-1 retries).
    let host = ScriptedHost::new([(
        "flaky",
        vec![
            Action::Err(DispatchError::DeterministicActionFailed {
                action: "flaky".into(),
                message: "1".into(),
            }),
            Action::Err(DispatchError::DeterministicActionFailed {
                action: "flaky".into(),
                message: "2".into(),
            }),
            Action::Err(DispatchError::DeterministicActionFailed {
                action: "flaky".into(),
                message: "3".into(),
            }),
        ],
    )]);
    let job = job_with_steps(vec![target_step_with_retry("flaky", "flaky", 3)]);
    let writer = std::sync::Arc::new(test_writer("run-retry-max"));
    let err = execute_job(&job, Value::Null, "run-retry-max", writer.clone(), &host)
        .expect_err("retry exhaustion must surface as Err");

    assert!(matches!(
        err,
        DispatchError::DeterministicActionFailed { ref message, .. } if message == "3"
    ));
    assert_eq!(host.call_count("flaky"), 3);
    let events = writer.events_snapshot().expect("audit");
    let retries: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            V2AuditEventKind::StepRetry { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(retries, vec![1, 2]);
}

#[test]
fn retry_stops_immediately_on_non_retryable_error() {
    // Invariant: `is_non_retryable()` errors (e.g. `ToolDenied`) skip the
    // retry loop and surface a `StepDenied` audit event.
    let host = ScriptedHost::new([(
        "denied",
        vec![Action::Err(DispatchError::ToolDenied {
            tool_name: "fs.write".into(),
            iteration: 1,
        })],
    )]);
    let job = job_with_steps(vec![target_step_with_retry("denied", "denied", 5)]);
    let writer = std::sync::Arc::new(test_writer("run-non-retryable"));

    let err = execute_job(
        &job,
        Value::Null,
        "run-non-retryable",
        writer.clone(),
        &host,
    )
    .expect_err("tool denial bubbles up");

    assert!(matches!(err, DispatchError::ToolDenied { .. }));
    assert_eq!(host.call_count("denied"), 1, "must not retry after denial");
    let events = writer.events_snapshot().expect("audit");
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.kind, V2AuditEventKind::StepDenied { .. })),
        "expected StepDenied audit event"
    );
}

#[test]
fn retry_returns_success_on_intermediate_attempt_without_extra_calls() {
    // Invariant: once an attempt succeeds, no further attempts run.
    let host = ScriptedHost::new([(
        "settle",
        vec![
            Action::Err(DispatchError::DeterministicActionFailed {
                action: "settle".into(),
                message: "1".into(),
            }),
            Action::Ok(json!({"settled": true})),
            Action::Ok(json!({"would-be-extra": true})),
        ],
    )]);
    let job = job_with_steps(vec![target_step_with_retry("settle", "settle", 5)]);

    let outcome = run_job(&host, &job, Value::Null, "run-retry-settle");

    assert!(outcome.success);
    assert_eq!(host.call_count("settle"), 2);
}

#[test]
fn compute_backoff_ms_respects_initial_max_and_zero_attempt_boundary() {
    // Invariant: `compute_backoff_ms` is monotonic with attempt index (linear
    // strategy) and never exceeds the cap. Pure unit test — no host required.
    let retry = RetrySpec {
        max_attempts: 5,
        initial_backoff_ms: 100,
        backoff_cap_ms: 250,
        backoff_strategy: BackoffStrategy::Linear,
    };
    // Linear: shifted = initial * (attempt_index + 1)
    assert_eq!(compute_backoff_ms(&retry, 0), 100); // 100 * 1
    assert_eq!(compute_backoff_ms(&retry, 1), 200); // 100 * 2
    assert_eq!(compute_backoff_ms(&retry, 2), 250); // 300 capped to 250
    assert_eq!(compute_backoff_ms(&retry, 5), 250); // capped

    let exp = RetrySpec {
        max_attempts: 5,
        initial_backoff_ms: 50,
        backoff_cap_ms: 1000,
        backoff_strategy: BackoffStrategy::Exponential,
    };
    assert_eq!(compute_backoff_ms(&exp, 0), 50); // 50 << 0
    assert_eq!(compute_backoff_ms(&exp, 1), 100); // 50 << 1
    assert_eq!(compute_backoff_ms(&exp, 4), 800); // 50 << 4
    assert_eq!(compute_backoff_ms(&exp, 10), 1000); // capped
}
