use std::path::Path;

use super::super::push::{is_valid_expected_remote_sha, push_args};

#[test]
fn force_push_uses_branch_scoped_exact_expected_sha_lease() {
    let expected = "0123456789abcdef0123456789abcdef01234567";
    let args = push_args(
        Path::new("/tmp/repo"),
        "origin",
        "orbit/task",
        Some(expected),
    );

    assert!(args.contains(&format!(
        "--force-with-lease=refs/heads/orbit/task:{expected}"
    )));
    assert!(!args.contains(&"--force-with-lease".to_string()));
}

#[test]
fn force_lease_requires_a_full_expected_remote_object_id() {
    assert!(!is_valid_expected_remote_sha(None));
    assert!(!is_valid_expected_remote_sha(Some("deadbeef")));
    assert!(is_valid_expected_remote_sha(Some(
        "0123456789abcdef0123456789abcdef01234567"
    )));
}
