use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn mcp_help_is_exposed() {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.arg("mcp")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("start"))
        .stdout(predicate::str::contains("init"));
}

#[test]
fn top_level_help_lists_mcp() {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("mcp"));
}
