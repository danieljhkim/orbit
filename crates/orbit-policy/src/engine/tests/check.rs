#![allow(missing_docs)]

//! Core `check` behavior: profile rule matching for allow/deny outcomes,
//! acceptance of normalized relative paths, and `matched_rule` reporting
//! for audit attribution.

use super::super::*;
use super::*;

#[test]
fn check_returns_allowed_when_path_inside_profile_read_rule() {
    // Invariant: a path matching a positive `read` rule resolves to
    // allowed=true with the matching rule recorded.
    let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("default", FsOperation::Read, "src/foo.rs")
        .expect("check");

    assert!(result.allowed);
    assert_eq!(result.matched_rule, "src/**");
}

#[test]
fn check_returns_denied_when_path_outside_modify_rules() {
    // Invariant: a Modify path that no positive rule matches resolves to
    // allowed=false. The matched_rule reflects the empty/no-match outcome
    // so the audit trail can attribute the deny.
    let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("default", FsOperation::Modify, "tests/foo.rs")
        .expect("check");

    assert!(!result.allowed);
    assert!(
        !result.matched_rule.is_empty(),
        "matched_rule must record the deny reason for audit attribution"
    );
}

#[test]
fn check_accepts_valid_relative_paths_after_normalization() {
    let def = make_def(
        vec![],
        vec![],
        &[("default", &["src/lib.rs"], &["src/lib.rs"])],
    );
    let engine = PolicyEngine::from_def(&def).expect("engine");

    for (operation, path) in [
        (FsOperation::Read, "src/lib.rs"),
        (FsOperation::Read, "./src/lib.rs"),
        (FsOperation::Modify, "src/lib.rs"),
        (FsOperation::Modify, "./src/lib.rs"),
    ] {
        let result = engine
            .check("default", operation, path)
            .expect("valid relative path should check");

        assert!(result.allowed, "{operation:?} `{path}` should be allowed");
        assert_eq!(result.matched_rule, "src/lib.rs");
    }
}

#[test]
fn check_records_matched_rule_for_audit_attribution() {
    // Invariant: a matched positive rule is reflected in the result's
    // `matched_rule` field so audit consumers can attribute the decision
    // to a specific rule rather than a bare allow/deny.
    let def = make_def(
        vec![],
        vec![],
        &[("default", &["src/lib.rs", "src/**"], &[])],
    );
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("default", FsOperation::Read, "src/lib.rs")
        .expect("check");
    assert!(result.allowed);
    assert!(
        result.matched_rule == "src/lib.rs" || result.matched_rule == "src/**",
        "matched_rule must surface a positive rule from the profile, got `{}`",
        result.matched_rule
    );
}
