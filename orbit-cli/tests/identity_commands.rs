use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd
}

fn orbit_in_with_home(dir: &Path, home: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd
}

fn write_identity(dir: &Path, id: &str, name: &str, role: &str) {
    let identity_root = dir.join(".orbit").join("identities");
    std::fs::create_dir_all(&identity_root).expect("create identity dir");
    let content = format!("identity:\n  name: {name}\n  role: {role}\n");
    std::fs::write(identity_root.join(format!("{id}.yaml")), content).expect("write identity");
}

fn write_raw_identity(dir: &Path, id: &str, content: &str) {
    let identity_root = dir.join(".orbit").join("identities");
    std::fs::create_dir_all(&identity_root).expect("create identity dir");
    std::fs::write(identity_root.join(format!("{id}.yaml")), content).expect("write identity");
}

fn write_identity_under(root: &Path, id: &str, name: &str, role: &str) {
    let identity_root = root.join(".orbit").join("identities");
    std::fs::create_dir_all(&identity_root).expect("create identity dir");
    let content = format!("identity:\n  name: {name}\n  role: {role}\n");
    std::fs::write(identity_root.join(format!("{id}.yaml")), content).expect("write identity");
}

#[test]
fn identity_list_shows_seeded_identities() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "alice", "Alice", "engineer");
    write_identity(dir.path(), "bob", "Bob", "member");

    orbit_in(dir.path())
        .args(["identity", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alice"))
        .stdout(predicate::str::contains("bob"));
}

#[test]
fn identity_list_json_is_valid() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "grace", "Grace", "engineer");

    let output = orbit_in(dir.path())
        .args(["identity", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert!(parsed.is_array());
    let ids: Vec<&str> = parsed
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert!(ids.contains(&"grace"));
}

#[test]
fn identity_list_is_deterministically_sorted() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "zara", "Zara", "member");
    write_identity(dir.path(), "alice", "Alice", "engineer");
    write_identity(dir.path(), "mike", "Mike", "member");

    let output = orbit_in(dir.path())
        .args(["identity", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    let lines: Vec<&str> = text.lines().collect();
    // skip header
    let data_lines: Vec<&str> = lines[1..].iter().copied().collect();
    let alice_pos = data_lines.iter().position(|l| l.contains("alice")).unwrap();
    let mike_pos = data_lines.iter().position(|l| l.contains("mike")).unwrap();
    let zara_pos = data_lines.iter().position(|l| l.contains("zara")).unwrap();
    assert!(alice_pos < mike_pos);
    assert!(mike_pos < zara_pos);
}

#[test]
fn identity_show_displays_identity_details() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "kent", "Kent", "engineer");

    orbit_in(dir.path())
        .args(["identity", "show", "kent"])
        .assert()
        .success()
        .stdout(predicate::str::contains("kent"))
        .stdout(predicate::str::contains("Kent"))
        .stdout(predicate::str::contains("engineer"));
}

#[test]
fn identity_show_json_is_valid() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "rob", "Rob", "leader");

    let output = orbit_in(dir.path())
        .args(["identity", "show", "rob", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(parsed["id"].as_str().unwrap(), "rob");
    assert_eq!(parsed["name"].as_str().unwrap(), "Rob");
    assert_eq!(parsed["role"].as_str().unwrap(), "leader");
}

#[test]
fn identity_show_unknown_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["identity", "show", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nonexistent"));
}

#[test]
fn identity_list_uses_workspace_identity_root_when_local_config_is_selected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    let home = dir.path().join("home");
    let workspace_orbit = workspace.join(".orbit");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace git dir");
    std::fs::create_dir_all(&workspace_orbit).expect("create workspace orbit dir");
    std::fs::create_dir_all(home.join(".orbit")).expect("create home orbit dir");
    std::fs::write(
        workspace_orbit.join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write workspace config");

    write_identity_under(&workspace, "prii", "Prii", "leader");

    orbit_in_with_home(&workspace, &home)
        .args(["identity", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("prii"));
}

#[test]
fn identity_list_fails_for_malformed_identity_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "alice", "Alice", "engineer");
    write_raw_identity(
        dir.path(),
        "broken",
        "identity:\n  name: Broken\n  role: [not-valid\n",
    );

    orbit_in(dir.path())
        .args(["identity", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid identity file"))
        .stderr(predicate::str::contains("broken.yaml"));
}

#[test]
fn identity_list_json_fails_for_malformed_identity_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_raw_identity(
        dir.path(),
        "broken",
        "identity:\n  name: Broken\n  role: [not-valid\n",
    );

    orbit_in(dir.path())
        .args(["identity", "list", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid identity file"))
        .stderr(predicate::str::contains("broken.yaml"));
}

#[test]
fn identity_list_role_filter_returns_only_matching_identities() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "alice", "Alice", "engineer");
    write_identity(dir.path(), "bob", "Bob", "member");
    write_identity(dir.path(), "carol", "Carol", "engineer");

    let output = orbit_in(dir.path())
        .args(["identity", "list", "--role", "engineer"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    assert!(text.contains("alice"), "alice (engineer) must appear");
    assert!(text.contains("carol"), "carol (engineer) must appear");
    assert!(!text.contains("bob"), "bob (member) must not appear");
}

#[test]
fn identity_list_role_filter_json_returns_only_matching_identities() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "alice", "Alice", "engineer");
    write_identity(dir.path(), "bob", "Bob", "member");

    let output = orbit_in(dir.path())
        .args(["identity", "list", "--role", "engineer", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    let ids: Vec<&str> = parsed
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert!(ids.contains(&"alice"));
    assert!(!ids.contains(&"bob"));
}

#[test]
fn identity_show_supports_architect_and_reviewer_roles() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "linus", "Linus", "engineer");
    write_identity(dir.path(), "lamport", "Lamport", "architect");
    write_identity(dir.path(), "prii", "Prii", "reviewer");

    orbit_in(dir.path())
        .args(["identity", "show", "lamport", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"role\": \"architect\""));

    orbit_in(dir.path())
        .args(["identity", "show", "prii", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"role\": \"reviewer\""));
}

#[test]
fn identity_list_role_filter_accepts_architect_and_reviewer() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "linus", "Linus", "engineer");
    write_identity(dir.path(), "lamport", "Lamport", "architect");
    write_identity(dir.path(), "prii", "Prii", "reviewer");

    orbit_in(dir.path())
        .args(["identity", "list", "--role", "architect"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lamport"))
        .stdout(predicate::str::contains("Lamport"));

    orbit_in(dir.path())
        .args(["identity", "list", "--role", "reviewer"])
        .assert()
        .success()
        .stdout(predicate::str::contains("prii"))
        .stdout(predicate::str::contains("Prii"));
}

#[test]
fn identity_list_role_filter_empty_match_returns_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_identity(dir.path(), "alice", "Alice", "engineer");

    let output = orbit_in(dir.path())
        .args(["identity", "list", "--role", "leader"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(output).expect("utf8");
    // Header line only, no data rows
    assert!(!text.contains("alice"));
}

#[test]
fn identity_list_role_filter_invalid_role_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["identity", "list", "--role", "not-a-real-role"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown identity role"));
}

#[test]
fn identity_help_shows_list_and_show_subcommands() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["identity", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("show"));
}
