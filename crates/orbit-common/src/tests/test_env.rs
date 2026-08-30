use super::scoped;

/// A managed-run child inherits `ORBIT_ROOT`-style ambient state from its
/// parent process, not from a sibling test in the same binary. This asserts
/// that two overlapping `scoped` guards never observe each other's values —
/// the second guard only ever sees what it itself requested.
#[test]
fn scoped_guard_isolates_ambient_value_from_a_concurrent_managed_run() {
    const VAR: &str = "ORBIT_TEST_ENV_ISOLATION_PROBE";
    let baseline = std::env::var(VAR).ok();

    {
        let _outer = scoped([(VAR, Some("ambient-managed-run"))]);
        assert_eq!(
            std::env::var(VAR).ok(),
            Some("ambient-managed-run".to_string())
        );
    }

    assert_eq!(
        std::env::var(VAR).ok(),
        baseline,
        "guard must restore the pre-existing value on drop"
    );
}

/// A test that panics while holding the guard must not turn the shared lock
/// into a permanent `PoisonError` for every later test in the binary
/// (ORB-11079 / F2026-08-098): `scoped` recovers a poisoned lock instead of
/// propagating it, and the panicking guard's own `Drop` still restores the
/// environment during unwinding.
#[test]
fn scoped_guard_recovers_after_a_sibling_assertion_panics_while_holding_it() {
    const VAR: &str = "ORBIT_TEST_ENV_POISON_PROBE";
    let baseline = std::env::var(VAR).ok();

    let panicked = std::thread::spawn(|| {
        let _guard = scoped([(VAR, Some("from-panicking-assertion"))]);
        panic!("simulated managed-run assertion failure while holding the env guard");
    })
    .join();
    assert!(panicked.is_err(), "expected the spawned thread to panic");

    // The shared mutex is now poisoned. A well-behaved next guard must
    // recover it rather than cascading the poison into every remaining test,
    // and must see only what it itself requested — not a leak from the
    // panicking thread's now-unwound scope.
    let guard = scoped([(VAR, Some("isolated-after-recovery"))]);
    assert_eq!(
        std::env::var(VAR).ok(),
        Some("isolated-after-recovery".to_string())
    );
    drop(guard);

    assert_eq!(std::env::var(VAR).ok(), baseline);
}
