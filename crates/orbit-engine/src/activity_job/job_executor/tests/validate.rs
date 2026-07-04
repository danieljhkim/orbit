#![allow(missing_docs)]

//! Retry-config validation invariants for `validate.rs` (ORB-10006).

use super::*;

fn retry(max_attempts: u32, initial_backoff_ms: u64, backoff_cap_ms: u64) -> RetrySpec {
    RetrySpec {
        max_attempts,
        initial_backoff_ms,
        backoff_cap_ms,
        backoff_strategy: BackoffStrategy::Exponential,
    }
}

fn job_with_retry(spec: RetrySpec) -> JobV2 {
    let mut step = target_step("build", "build");
    step.retry = Some(spec);
    job_with_steps(vec![step])
}

#[test]
fn validate_job_accepts_well_formed_retry_config() {
    for spec in [
        retry(1, 1, 1),
        retry(3, 100, 100),
        retry(4, 200, 2_000),
        retry(10, 1_000, 60_000),
    ] {
        let job = job_with_retry(spec.clone());
        assert!(
            validate_job(&job).is_ok(),
            "expected valid retry config to pass: {spec:?}"
        );
    }
}

#[test]
fn validate_job_rejects_zero_max_attempts() {
    let err = validate_job(&job_with_retry(retry(0, 100, 1_000)))
        .expect_err("zero max_attempts must be rejected");
    match err {
        DispatchError::RetryConfigInvalid {
            step_id,
            field,
            value,
            ..
        } => {
            assert_eq!(step_id, "build");
            assert_eq!(field, "max_attempts");
            assert_eq!(value, 0);
        }
        other => panic!("expected RetryConfigInvalid, got {other:?}"),
    }
}

#[test]
fn validate_job_rejects_zero_initial_backoff() {
    let err = validate_job(&job_with_retry(retry(3, 0, 1_000)))
        .expect_err("zero initial_backoff_ms must be rejected");
    match err {
        DispatchError::RetryConfigInvalid { field, value, .. } => {
            assert_eq!(field, "initial_backoff_ms");
            assert_eq!(value, 0);
        }
        other => panic!("expected RetryConfigInvalid, got {other:?}"),
    }
}

#[test]
fn validate_job_rejects_cap_below_initial_backoff_naming_both_values() {
    let err = validate_job(&job_with_retry(retry(3, 500, 100)))
        .expect_err("inverted cap must be rejected");
    match &err {
        DispatchError::RetryConfigInvalid {
            field,
            value,
            invariant,
            ..
        } => {
            assert_eq!(*field, "backoff_cap_ms");
            assert_eq!(*value, 100);
            assert!(
                invariant.contains("500"),
                "invariant should name initial_backoff_ms: {invariant}"
            );
        }
        other => panic!("expected RetryConfigInvalid, got {other:?}"),
    }
    // The rendered message names the offending field and value.
    let rendered = err.to_string();
    assert!(rendered.contains("backoff_cap_ms"), "message: {rendered}");
    assert!(rendered.contains("100"), "message: {rendered}");
    assert!(rendered.contains("500"), "message: {rendered}");
}

#[test]
fn validate_job_checks_retry_config_in_nested_blocks() {
    // Retry invariants are enforced recursively (parallel branches, loop
    // bodies, fan-out workers), not just on top-level steps.
    let mut nested = target_step("inner", "inner");
    nested.retry = Some(retry(2, 300, 10));
    let parallel = JobV2Step {
        id: "outer".to_string(),
        when: None,
        retry: None,
        recovery_activity: None,
        resolved_recovery_activity: None,
        body: JobV2StepBody::Parallel {
            parallel: ParallelBlock {
                join: JoinMode::All,
                branches: vec![nested],
            },
        },
    };
    let err = validate_job(&job_with_steps(vec![parallel]))
        .expect_err("nested invalid retry config must be rejected");
    assert!(
        matches!(
            err,
            DispatchError::RetryConfigInvalid { ref step_id, .. } if step_id == "inner"
        ),
        "got {err:?}"
    );
}

#[test]
fn retry_config_invalid_is_non_retryable() {
    let err = DispatchError::RetryConfigInvalid {
        step_id: "s".into(),
        field: "backoff_cap_ms",
        value: 1,
        invariant: "backoff_cap_ms >= initial_backoff_ms (2)".into(),
    };
    assert!(err.is_non_retryable());
}
