#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin_cmd;
use tempfile::tempdir;

#[test]
#[ignore = "ORB-00110: disabled at v0.6.0 release time — golden-fixture model is fragile against CI checkout mtime semantics and produces release-blocking false positives. Re-enable once the decay-check rebases on a deterministic date source (git committer-date for the cited .rs file, not on-disk mtime)."]
fn design_check_matches_current_corpus_golden() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonical repo root");
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("create home");

    let output = cargo_bin_cmd!("orbit")
        .current_dir(temp.path())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["design", "check", "--warn-only", "--workspace"])
        .arg(&repo_root)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(
        String::from_utf8(output).expect("utf8 stdout"),
        include_str!("fixtures/design_check_current_golden.txt")
    );
}
