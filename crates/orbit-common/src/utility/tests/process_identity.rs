mod identity {
    use super::super::super::process_identity::*;

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
}

#[cfg(unix)]
mod liveness {
    use super::super::super::process_identity::*;

    #[test]
    fn self_pid_is_alive_with_and_without_a_matching_token() {
        let pid = std::process::id();
        assert!(process_is_alive(pid));
        assert_eq!(probe_process_liveness(pid, None), ProcessLiveness::Alive);

        let Some(token) = process_start_identity_token(pid) else {
            // `ps` unavailable in this sandbox; the untokened assertion above
            // is the part that still means something here.
            return;
        };
        assert_eq!(
            probe_process_liveness(pid, Some(&token)),
            ProcessLiveness::Alive
        );
    }

    #[test]
    fn live_pid_with_a_foreign_identity_token_reads_as_exited() {
        // The PID-reuse case: the number is alive, but it is not the process
        // that was recorded. Reporting it alive would tell an operator their
        // dead agent is still working.
        let pid = std::process::id();
        assert_eq!(
            probe_process_liveness(pid, Some("ps-lstart-utc-v1:Thu Jan  1 00:00:00 1970")),
            ProcessLiveness::Exited
        );
    }

    #[test]
    fn missing_pid_reads_as_exited() {
        assert!(!process_is_alive(u32::MAX - 1));
        assert_eq!(
            probe_process_liveness(u32::MAX - 1, None),
            ProcessLiveness::Exited
        );
        assert!(!process_is_alive(0));
    }

    #[test]
    fn liveness_tokens_are_stable() {
        assert_eq!(ProcessLiveness::Alive.as_str(), "alive");
        assert_eq!(ProcessLiveness::Exited.as_str(), "exited");
        assert_eq!(ProcessLiveness::Unknown.as_str(), "unknown");
    }
}
