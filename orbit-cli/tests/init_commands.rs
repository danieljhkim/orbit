use assert_cmd::Command;
use predicates::prelude::*;

fn orbit_in(dir: &std::path::Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd
}

#[test]
fn init_creates_default_identities_under_home_orbit() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized identities"));

    let identity_root = home.path().join(".orbit").join("identities");
    assert!(identity_root.join("linus.yaml").exists());
    assert!(identity_root.join("john.yaml").exists());
    assert!(identity_root.join("kent.yaml").exists());
    assert!(identity_root.join("rob.yaml").exists());
    assert!(identity_root.join("grace.yaml").exists());
    assert!(identity_root.join("steve.yaml").exists());
}

#[test]
fn init_is_idempotent_for_existing_identity_files() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created=6"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created=0"));
}
