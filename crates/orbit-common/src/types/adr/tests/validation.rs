use super::super::*;

#[test]
fn validate_adr_id_accepts_canonical_ids() {
    validate_adr_id("ADR-0001").expect("ADR-0001 should be valid");
    validate_adr_id("ADR-9999").expect("ADR-9999 should be valid");
    validate_adr_id("ADR-12345").expect("ADR-12345 (5 digits) should be valid");
}

#[test]
fn validate_adr_id_rejects_invalid_ids() {
    assert!(validate_adr_id("").is_err(), "empty should be rejected");
    assert!(
        validate_adr_id("ADR-1").is_err(),
        "1 digit should be rejected"
    );
    assert!(
        validate_adr_id("ADR-001").is_err(),
        "3 digits should be rejected"
    );
    assert!(
        validate_adr_id("adr-0001").is_err(),
        "lowercase prefix should be rejected"
    );
    assert!(
        validate_adr_id("ADR-XXXX").is_err(),
        "non-digit suffix should be rejected"
    );
}

#[test]
fn legacy_id_for_pads_local_number_to_three_digits() {
    assert_eq!(legacy_id_for("activity-job", 17), "activity-job/ADR-017");
}
