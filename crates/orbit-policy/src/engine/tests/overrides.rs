#![allow(missing_docs)]

//! Global `denyRead` / `denyModify` precedence (last-match-wins) over
//! profile-level positive `read`/`modify` rules.

use super::super::*;
use super::*;

#[test]
fn check_global_deny_modify_overrides_profile_modify_allow() {
    // Invariant (CLAUDE.md "global denyModify rules accumulate"): a
    // global `denyModify` rule must beat a profile-level positive
    // `modify` rule under last-match-wins evaluation.
    let def = make_def(
        vec![],
        vec!["src/secrets/**"],
        &[("default", &["src/**"], &["src/**"])],
    );
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("default", FsOperation::Modify, "src/secrets/key.txt")
        .expect("check");

    assert!(
        !result.allowed,
        "global denyModify must override profile-level modify allow"
    );
}

#[test]
fn check_global_deny_read_overrides_profile_read_allow() {
    let def = make_def(
        vec!["src/secrets/**"],
        vec![],
        &[("default", &["src/**"], &["src/**"])],
    );
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("default", FsOperation::Read, "src/secrets/key.txt")
        .expect("check");

    assert!(
        !result.allowed,
        "global denyRead must override profile-level read allow"
    );
}
