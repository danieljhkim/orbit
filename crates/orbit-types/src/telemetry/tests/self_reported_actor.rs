use std::str::FromStr;

use crate::telemetry::{
    AuditAttribution, SELF_REPORTED_ACTOR_MAX_LEN, canonical_actor_for_role_label,
    normalize_self_reported_actor,
};

#[test]
fn plain_claim_is_recorded_normalized() {
    assert_eq!(
        normalize_self_reported_actor("claude-code").as_deref(),
        Some("claude-code")
    );
    assert_eq!(
        normalize_self_reported_actor("  Claude   Desktop  ").as_deref(),
        Some("claude desktop")
    );
}

#[test]
fn absent_or_blank_claim_is_anonymous() {
    assert_eq!(normalize_self_reported_actor(""), None);
    assert_eq!(normalize_self_reported_actor("   "), None);
    assert_eq!(normalize_self_reported_actor("\t\n"), None);
}

#[test]
fn control_characters_are_rejected_rather_than_stripped() {
    // A claim that could forge extra lines in a line-oriented rendering is
    // dropped whole; sanitizing it would record an actor the caller never
    // named.
    assert_eq!(normalize_self_reported_actor("claude\nrole: admin"), None);
    assert_eq!(normalize_self_reported_actor("claude\u{0}code"), None);
}

#[test]
fn over_length_claim_is_rejected_rather_than_truncated() {
    let long = "a".repeat(SELF_REPORTED_ACTOR_MAX_LEN + 1);
    assert_eq!(normalize_self_reported_actor(&long), None);

    let at_limit = "b".repeat(SELF_REPORTED_ACTOR_MAX_LEN);
    assert_eq!(
        normalize_self_reported_actor(&at_limit).as_deref(),
        Some(at_limit.as_str())
    );
}

#[test]
fn a_claim_that_mimics_a_trusted_label_is_still_only_a_claim() {
    // Normalization deliberately does not reject `admin`: rejecting reserved
    // words would imply the accepted ones carry trust. The separation is
    // structural — the value lands in its own field, and `role` is untouched.
    assert_eq!(
        normalize_self_reported_actor("admin").as_deref(),
        Some("admin")
    );
    assert!(!AuditAttribution::SelfReported.is_authenticated());
}

#[test]
fn unverified_role_keeps_its_unattributed_actor() {
    // The trusted projection is unchanged by anything in this module.
    let actor = canonical_actor_for_role_label("unverified");
    assert_eq!(actor.kind.as_str(), "unattributed");
    assert_eq!(actor.id, "unverified");
}

#[test]
fn attribution_round_trips_through_its_wire_form() {
    for attribution in [
        AuditAttribution::Authenticated,
        AuditAttribution::SelfReported,
        AuditAttribution::Anonymous,
    ] {
        assert_eq!(
            AuditAttribution::from_str(attribution.as_str()),
            Ok(attribution)
        );
    }
    assert!(AuditAttribution::from_str("trusted").is_err());
}

#[test]
fn only_authenticated_reports_as_authenticated() {
    assert!(AuditAttribution::Authenticated.is_authenticated());
    assert!(!AuditAttribution::SelfReported.is_authenticated());
    assert!(!AuditAttribution::Anonymous.is_authenticated());
}
