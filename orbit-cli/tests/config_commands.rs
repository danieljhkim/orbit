use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd
}

#[test]
fn config_show_json_bootstraps_orbit_home_when_missing() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], true);
    assert_eq!(value["execution"]["env"]["inherit"], false);
    assert_eq!(
        value["execution"]["env"]["pass"],
        serde_json::json!(["CODEX_HOME", "HOME", "PATH"])
    );
    assert_eq!(
        value["task"]["approval"]["required_for_agent"],
        serde_json::json!(true)
    );
    assert_eq!(
        value["persistence"]["scheduler"]["persistence"]["type"],
        serde_json::json!("file")
    );
    assert_eq!(
        value["persistence"]["job"]["persistence"]["type"],
        serde_json::json!("file")
    );
    assert_eq!(
        value["persistence"]["watch"]["persistence"]["type"],
        serde_json::json!("sqlite")
    );
    assert_eq!(
        value["persistence"]["audit"]["persistence"]["type"],
        serde_json::json!("sqlite")
    );

    assert!(dir.path().join(".orbit").join("config.toml").exists());
    assert!(
        dir.path()
            .join(".orbit")
            .join("identities")
            .join("linus.yaml")
            .exists()
    );
    assert!(
        dir.path()
            .join(".orbit")
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );
}

#[test]
fn config_show_json_reads_and_normalizes_runtime_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[execution.env]\ninherit = true\npass = [\"PATH\",\"HOME\",\"PATH\"]\n\n[task.approval]\nrequired_for_agent = true\n\n[scheduler]\npersistence = { type = \"sqlite\", path = \"./.orbit/orbit.db\" }\n",
    )
    .expect("write config");

    let output = orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], true);
    assert_eq!(value["execution"]["env"]["inherit"], true);
    assert_eq!(
        value["execution"]["env"]["pass"],
        serde_json::json!(["HOME", "PATH"])
    );
    assert_eq!(
        value["task"]["approval"]["required_for_agent"],
        serde_json::json!(true)
    );
    assert_eq!(
        value["persistence"]["scheduler"]["persistence"]["type"],
        serde_json::json!("sqlite")
    );
}

#[test]
fn config_show_json_reports_workspace_config_path_when_local_config_is_used() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    let home = dir.path().join("home");
    let local_orbit_dir = workspace.join(".orbit");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace git dir");
    std::fs::create_dir_all(&local_orbit_dir).expect("create workspace orbit dir");
    std::fs::create_dir_all(home.join(".orbit")).expect("create home orbit dir");

    std::fs::write(
        local_orbit_dir.join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write workspace config");

    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(&workspace);
    cmd.env("HOME", &home);
    cmd.env("USERPROFILE", &home);

    let output = cmd
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    let expected_path = std::fs::canonicalize(local_orbit_dir.join("config.toml"))
        .expect("canonical workspace config");
    let reported_path = std::fs::canonicalize(
        value["path"]
            .as_str()
            .expect("path should be a string in config show json"),
    )
    .expect("canonical reported path");
    assert_eq!(reported_path, expected_path);
    assert_eq!(value["exists"], serde_json::json!(true));
    assert_eq!(
        value["task"]["approval"]["required_for_agent"],
        serde_json::json!(true)
    );
}

#[test]
fn non_init_commands_in_repo_bootstrap_only_home_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    let home = dir.path().join("home");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace git dir");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    std::fs::create_dir_all(&home).expect("create home dir");

    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(&workspace);
    cmd.env("HOME", &home);
    cmd.env("USERPROFILE", &home);

    let output = cmd
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], serde_json::json!(true));
    assert!(home.join(".orbit").join("config.toml").exists());
    assert!(!workspace.join(".orbit").exists());
}
