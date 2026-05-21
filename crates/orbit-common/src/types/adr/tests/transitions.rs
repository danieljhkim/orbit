use super::super::*;

#[test]
fn transition_proposed_to_accepted_is_allowed() {
    AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Accepted)
        .expect("proposed -> accepted should be allowed");
}

#[test]
fn transition_proposed_to_superseded_is_allowed() {
    AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Superseded)
        .expect("proposed -> superseded should be allowed");
}

#[test]
fn transition_proposed_to_deleted_is_allowed() {
    AdrStatus::validate_transition(AdrStatus::Proposed, AdrStatus::Deleted)
        .expect("proposed -> deleted should be allowed");
}

#[test]
fn transition_accepted_to_superseded_is_allowed() {
    AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Superseded)
        .expect("accepted -> superseded should be allowed");
}

#[test]
fn transition_same_state_is_idempotent_for_all_variants() {
    for status in [
        AdrStatus::Proposed,
        AdrStatus::Accepted,
        AdrStatus::Superseded,
        AdrStatus::Deleted,
    ] {
        AdrStatus::validate_transition(status, status)
            .expect("same-state transition should be idempotent");
    }
}

#[test]
fn transition_accepted_to_proposed_is_rejected() {
    let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Proposed)
        .expect_err("accepted -> proposed should be rejected");
    assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
    assert!(err.to_string().contains("accepted"));
    assert!(err.to_string().contains("proposed"));
}

#[test]
fn transition_accepted_to_deleted_is_rejected() {
    let err = AdrStatus::validate_transition(AdrStatus::Accepted, AdrStatus::Deleted)
        .expect_err("accepted -> deleted should be rejected");
    assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
    assert!(err.to_string().contains("superseded"));
}

#[test]
fn transition_superseded_to_anything_is_rejected() {
    for target in [AdrStatus::Proposed, AdrStatus::Accepted, AdrStatus::Deleted] {
        let err = AdrStatus::validate_transition(AdrStatus::Superseded, target)
            .expect_err("superseded is terminal");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("terminal"));
    }
}

#[test]
fn transition_deleted_to_anything_is_rejected() {
    for target in [
        AdrStatus::Proposed,
        AdrStatus::Accepted,
        AdrStatus::Superseded,
    ] {
        let err = AdrStatus::validate_transition(AdrStatus::Deleted, target)
            .expect_err("deleted is terminal");
        assert!(matches!(err, OrbitError::AdrInvalidTransition(_)));
        assert!(err.to_string().contains("terminal"));
    }
}
