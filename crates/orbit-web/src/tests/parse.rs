use chrono::Utc;

use super::super::parse::*;

#[test]
fn parses_rfc3339() {
    // test-only: unwrap is acceptable for deterministic RFC3339 input in an isolated unit test
    #[allow(clippy::unwrap_used)]
    let ts = parse_since("2025-01-01T00:00:00Z").unwrap();
    assert_eq!(ts.timestamp(), 1735689600);
}

#[test]
fn parses_duration() {
    // This is a bit racy on "now", but we can assert it's recent.
    // test-only: unwrap is acceptable for deterministic duration input in an isolated unit test
    #[allow(clippy::unwrap_used)]
    let ts = parse_since("10s").unwrap();
    let now = Utc::now();
    assert!(now.signed_duration_since(ts).num_seconds() >= 9);
}
