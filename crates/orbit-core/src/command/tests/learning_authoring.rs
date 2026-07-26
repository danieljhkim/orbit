//! Tests for `crates/orbit-core/src/command/learning_authoring.rs` [ORB-10364].
//!
//! Role derivation reads process-global environment, so every test here takes
//! the shared [`orbit_common::test_env`] lock via `scoped_identity_env` before
//! mutating it. End-to-end coverage of the CLI and `orbit.learning.*` tool
//! surfaces (which spawn a child with an explicit environment and need no
//! lock) lives in `crates/orbit-cli/tests/learning.rs`.

use orbit_common::test_env::{AGENT_IDENTITY_ENV, ScopedEnv, unset};
use orbit_common::types::OrbitError;

use crate::command::learning_authoring::{
    LEARNING_AUTHOR_OPT_IN_ENV, LearningAuthorRole, LearningWriteAttempt,
    ensure_learning_write_allowed, learning_author_role,
};

/// Clear the identity pair *and* the opt-in for the guard's lifetime, then
/// apply `vars` on top. Holding one guard for all of them keeps the process
/// env consistent for the whole test.
fn scoped_identity_env(vars: &[(&str, &str)]) -> ScopedEnv {
    let guard = unset(
        AGENT_IDENTITY_ENV
            .iter()
            .copied()
            .chain(std::iter::once(LEARNING_AUTHOR_OPT_IN_ENV)),
    );
    // SAFETY: `guard` holds the process-wide env lock and restores every name
    // it cleared — including the ones set here — when it drops.
    unsafe {
        for (name, value) in vars {
            std::env::set_var(name, value);
        }
    }
    guard
}

fn add_attempt() -> LearningWriteAttempt<'static> {
    LearningWriteAttempt::Add {
        summary: "always re-run the projection after a rename",
        body: "the index keeps the stale row until sync",
    }
}

fn denial_message(result: Result<(), OrbitError>) -> String {
    match result {
        Err(OrbitError::PolicyDenied(message)) => message,
        Err(error) => panic!("expected a policy denial, got {error:?}"),
        Ok(()) => panic!("expected a policy denial, got success"),
    }
}

#[test]
fn no_agent_identity_env_preserves_the_existing_authoring_policy() {
    let _env = scoped_identity_env(&[]);

    assert_eq!(learning_author_role(), LearningAuthorRole::Human);
    assert!(ensure_learning_write_allowed(add_attempt()).is_ok());
    assert!(
        ensure_learning_write_allowed(LearningWriteAttempt::Update {
            id: "L-0001",
            summary: Some("rewrite"),
            body: None,
        })
        .is_ok()
    );
    assert!(
        ensure_learning_write_allowed(LearningWriteAttempt::Supersede {
            id: "L-0001",
            with: "L-0002",
        })
        .is_ok()
    );
}

#[test]
fn agent_identity_env_without_opt_in_resolves_to_the_executor_role() {
    let _env = scoped_identity_env(&[("ORBIT_AGENT_MODEL", "claude-opus-5")]);

    assert_eq!(
        learning_author_role(),
        LearningAuthorRole::Executor {
            label: "claude".to_string()
        }
    );
}

/// `ORBIT_AGENT_NAME` alone is enough — the runtime's identity derivation
/// prefers the model but falls back to the name, and either one means "agent".
#[test]
fn agent_name_alone_also_resolves_to_the_executor_role() {
    let _env = scoped_identity_env(&[("ORBIT_AGENT_NAME", "claude")]);

    assert_eq!(
        learning_author_role(),
        LearningAuthorRole::Executor {
            label: "claude".to_string()
        }
    );
}

#[test]
fn executor_add_is_refused_with_the_friction_redirect_and_the_attempted_payload() {
    let _env = scoped_identity_env(&[("ORBIT_AGENT_MODEL", "claude-opus-5")]);

    let message = denial_message(ensure_learning_write_allowed(add_attempt()));

    assert!(message.contains("learning add"), "names the operation");
    assert!(message.contains("claude"), "names the canonical identity");
    assert!(
        message.contains("orbit friction add") && message.contains("orbit.friction.add"),
        "redirects to the friction surface: {message}"
    );
    assert!(
        message.contains("always re-run the projection after a rename")
            && message.contains("the index keeps the stale row until sync"),
        "echoes the attempted content: {message}"
    );
    assert!(
        message.contains(LEARNING_AUTHOR_OPT_IN_ENV),
        "names the orchestrator opt-in: {message}"
    );
}

#[test]
fn executor_update_and_supersede_are_refused_and_echo_their_own_payloads() {
    let _env = scoped_identity_env(&[("ORBIT_AGENT_MODEL", "claude-opus-5")]);

    let update = denial_message(ensure_learning_write_allowed(
        LearningWriteAttempt::Update {
            id: "L-0082",
            summary: Some("narrowed scope"),
            body: None,
        },
    ));
    assert!(update.contains("learning update"));
    assert!(update.contains("L-0082") && update.contains("narrowed scope"));
    // An omitted field is not echoed as an empty one.
    assert!(
        !update.contains("body:"),
        "no body line when unset: {update}"
    );

    let supersede = denial_message(ensure_learning_write_allowed(
        LearningWriteAttempt::Supersede {
            id: "L-0108",
            with: "L-0109",
        },
    ));
    assert!(supersede.contains("learning supersede"));
    assert!(supersede.contains("L-0108") && supersede.contains("L-0109"));
}

#[test]
fn a_long_body_is_truncated_in_the_echo_rather_than_dropped() {
    let body = "x".repeat(4000);
    let _env = scoped_identity_env(&[("ORBIT_AGENT_MODEL", "claude-opus-5")]);

    let message = denial_message(ensure_learning_write_allowed(LearningWriteAttempt::Add {
        summary: "long one",
        body: &body,
    }));

    assert!(message.contains("[truncated]"), "marks the truncation");
    assert!(message.len() < body.len(), "does not echo the whole body");
    assert!(message.contains(&"x".repeat(1500)), "keeps the head");
}

#[test]
fn the_explicit_orchestrator_opt_in_allows_agent_context_writes() {
    for truthy in ["1", "true", "TRUE"] {
        let _env = scoped_identity_env(&[
            ("ORBIT_AGENT_MODEL", "claude-opus-5"),
            (LEARNING_AUTHOR_OPT_IN_ENV, truthy),
        ]);

        assert_eq!(
            learning_author_role(),
            LearningAuthorRole::AuthorizedAgent {
                label: "claude".to_string()
            },
            "`{truthy}` opts in"
        );
        assert!(ensure_learning_write_allowed(add_attempt()).is_ok());
    }
}

/// The opt-in must be deliberate: anything that is not an affirmative value
/// leaves the caller an executor rather than silently bypassing the gate.
#[test]
fn a_non_affirmative_opt_in_value_does_not_bypass_the_gate() {
    for falsy in ["", "0", "false", "no", "yes-please"] {
        let _env = scoped_identity_env(&[
            ("ORBIT_AGENT_MODEL", "claude-opus-5"),
            (LEARNING_AUTHOR_OPT_IN_ENV, falsy),
        ]);

        assert!(
            matches!(learning_author_role(), LearningAuthorRole::Executor { .. }),
            "`{falsy}` must not opt in"
        );
        assert!(ensure_learning_write_allowed(add_attempt()).is_err());
    }
}

/// The opt-in alone is not an identity. The runtime actor remains unknown,
/// while this task deliberately preserves the existing authoring policy.
#[test]
fn the_opt_in_without_agent_identity_is_still_the_human_role() {
    let _env = scoped_identity_env(&[(LEARNING_AUTHOR_OPT_IN_ENV, "1")]);

    assert_eq!(learning_author_role(), LearningAuthorRole::Human);
    assert!(ensure_learning_write_allowed(add_attempt()).is_ok());
}
