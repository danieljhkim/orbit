use super::super::*;

#[test]
fn token_is_versioned_and_stable_for_self_pid() {
    let pid = std::process::id();
    let outcome = probe_process_start_identity(pid);
    let ProbeOutcome::Token(first) = outcome else {
        return;
    };
    assert!(
        first.starts_with(STABLE_TOKEN_PREFIX),
        "token must carry the versioned prefix: {first}"
    );
    let second = process_start_identity_token(pid).expect("second token");
    assert_eq!(first, second, "stable token must be deterministic");
}

#[test]
fn legacy_match_rejects_versioned_input() {
    let pid = std::process::id();
    let Some(versioned) = process_start_identity_token(pid) else {
        return;
    };
    assert!(
        !legacy_lstart_matches(pid, &versioned),
        "versioned tokens must not be accepted via the legacy path"
    );
}

#[test]
fn dead_pid_yields_no_process_probe_outcome() {
    // PIDs near u32::MAX cannot exist on any supported platform; `ps`
    // returns non-zero or errors, yielding NoProcess or Unavailable depending
    // on platform ps(1) behavior. Accept either as "definitely not running".
    let outcome = probe_process_start_identity(u32::MAX - 1);
    assert!(
        matches!(outcome, ProbeOutcome::NoProcess | ProbeOutcome::Unavailable),
        "expected terminal outcome for dead pid, got {outcome:?}"
    );
    assert!(process_start_identity_token(u32::MAX - 1).is_none());
    assert!(!legacy_lstart_matches(u32::MAX - 1, "anything"));
}
