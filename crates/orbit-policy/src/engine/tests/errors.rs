#![allow(missing_docs)]

//! Error and special-case paths for `check`: parent traversal rejection
//! (security), unknown profile surfacing `InvalidInput`, and the documented
//! `unrestricted` profile fallback that bypasses the unknown-profile error.

use super::super::*;
use super::*;

#[test]
fn check_rejects_parent_traversal_for_read_and_modify_paths() {
    let def = make_def(vec![], vec![], &[("default", &["**"], &["**"])]);
    let engine = PolicyEngine::from_def(&def).expect("engine");

    for (operation, path) in [
        (FsOperation::Read, "../secret.txt"),
        (FsOperation::Read, "src/../secret.txt"),
        (FsOperation::Read, "..\\secret.txt"),
        (FsOperation::Read, "src\\..\\secret.txt"),
        (FsOperation::Modify, "../secret.txt"),
        (FsOperation::Modify, "src/../secret.txt"),
        (FsOperation::Modify, "..\\secret.txt"),
        (FsOperation::Modify, "src\\..\\secret.txt"),
    ] {
        let err = engine
            .check("default", operation, path)
            .expect_err("parent traversal must be rejected");

        assert!(
            matches!(err, OrbitError::InvalidInput(_)),
            "expected InvalidInput for {operation:?} `{path}`, got {err:?}"
        );
    }
}

#[test]
fn check_unknown_profile_returns_error_not_silent_allow() {
    // Invariant: requesting an undefined profile name must surface a
    // structured error rather than silently allowing or silently denying.
    // (The `unrestricted` profile is a documented special case;
    // arbitrary names must not be.)
    let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let err = engine
        .check("missing", FsOperation::Read, "src/foo.rs")
        .expect_err("unknown profile must error");

    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn check_unknown_profile_resolves_unrestricted_when_named_unrestricted() {
    // Invariant: the special `unrestricted` profile resolves to the
    // documented permissive defaults even when the policy doesn't define
    // it. This is the single named exception to the unknown-profile
    // error path.
    let def = make_def(vec![], vec![], &[]);
    let engine = PolicyEngine::from_def(&def).expect("engine");

    let result = engine
        .check("unrestricted", FsOperation::Read, "anywhere.rs")
        .expect("unrestricted profile resolves");
    assert!(result.allowed);
}
