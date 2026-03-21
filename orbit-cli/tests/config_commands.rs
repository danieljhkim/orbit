use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd.env("ORBIT_ROOT", dir.join(".orbit"));
    cmd
}

#[test]
fn config_show_json_bootstraps_cwd_orbit_when_missing() {
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
    assert_eq!(value["root"], value["selected_root"]);
    assert!(
        value
            .as_object()
            .expect("config object")
            .get("home")
            .is_none()
    );
    assert_eq!(value["execution"]["env"]["inherit"], false);
    let expected_pass = if cfg!(target_os = "macos") {
        serde_json::json!([
            "CODEX_HOME",
            "HOME",
            "PATH",
            "TMPDIR",
            "USER",
            "__CF_USER_TEXT_ENCODING"
        ])
    } else {
        serde_json::json!(["CODEX_HOME", "HOME", "PATH", "TMPDIR", "USER"])
    };
    assert_eq!(value["execution"]["env"]["pass"], expected_pass);
    assert_eq!(
        value["execution"]["codex"]["sandbox"],
        serde_json::json!("workspace-write")
    );
    assert_eq!(
        value["execution"]["codex"]["approval_policy"],
        serde_json::Value::Null
    );
    assert_eq!(
        value["task"]["approval"]["required_for_agent"],
        serde_json::json!(true)
    );
    assert_eq!(
        value["persistence"]["job"]["persistence"]["type"],
        serde_json::json!("file")
    );
    assert_eq!(
        value["persistence"]["activity"]["persistence"]["type"],
        serde_json::json!("file")
    );
    assert_eq!(
        value["persistence"]["audit"]["persistence"]["type"],
        serde_json::json!("sqlite")
    );
    assert!(
        value["persistence"]
            .as_object()
            .expect("persistence object")
            .get("watch")
            .is_none()
    );

    assert!(dir.path().join(".orbit").join("config.toml").exists());
    assert!(
        dir.path()
            .join(".orbit")
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );
    let config_raw = std::fs::read_to_string(dir.path().join(".orbit").join("config.toml"))
        .expect("read config");
    assert!(!config_raw.contains("[watch]"));
    assert!(config_raw.contains("[execution.codex]"));
}

#[test]
fn config_show_json_reads_and_normalizes_runtime_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[execution.env]\ninherit = true\npass = [\"PATH\",\"HOME\",\"PATH\"]\n\n[execution.codex]\nsandbox = \"danger-full-access\"\napproval_policy = \"on-request\"\n\n[task.approval]\nrequired_for_agent = true\n\n[job]\npersistence = { type = \"file\", path = \"./custom-jobs\", format = \"yaml\" }\n",
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
    assert_eq!(value["root"], value["selected_root"]);
    assert!(
        value
            .as_object()
            .expect("config object")
            .get("home")
            .is_none()
    );
    assert_eq!(value["execution"]["env"]["inherit"], true);
    assert_eq!(
        value["execution"]["env"]["pass"],
        serde_json::json!(["HOME", "PATH"])
    );
    assert_eq!(
        value["execution"]["codex"]["sandbox"],
        serde_json::json!("danger-full-access")
    );
    assert_eq!(
        value["execution"]["codex"]["approval_policy"],
        serde_json::json!("on-request")
    );
    assert_eq!(
        value["task"]["approval"]["required_for_agent"],
        serde_json::json!(true)
    );
    assert_eq!(
        value["persistence"]["job"]["persistence"]["type"],
        serde_json::json!("file")
    );
    let reported_jobs_path = std::fs::canonicalize(
        value["persistence"]["job"]["persistence"]["path"]
            .as_str()
            .expect("job persistence path"),
    )
    .expect("canonical reported jobs path");
    let expected_jobs_path =
        std::fs::canonicalize(orbit_dir.join("custom-jobs")).expect("canonical expected jobs path");
    assert_eq!(reported_jobs_path, expected_jobs_path);
}

#[test]
fn config_show_rejects_sqlite_jobs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[job]\npersistence = { type = \"sqlite\", path = \"./.orbit/orbit.db\" }\n",
    )
    .expect("write config");

    orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "job.persistence.type only supports 'file'",
        ));
}

#[test]
fn config_show_rejects_sqlite_activities() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[activity]\npersistence = { type = \"sqlite\", path = \"./.orbit/orbit.db\" }\n",
    )
    .expect("write config");

    orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "activity.persistence.type only supports 'file'",
        ));
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
    cmd.env("ORBIT_ROOT", &local_orbit_dir);

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
    assert_eq!(value["root"], value["selected_root"]);
    assert!(
        value
            .as_object()
            .expect("config object")
            .get("home")
            .is_none()
    );
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
fn non_init_commands_bootstrap_global_root() {
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
    cmd.env("ORBIT_ROOT", workspace.join(".orbit"));

    let output = cmd
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&output).expect("json");

    assert_eq!(value["exists"], serde_json::json!(true));
    // Global root (~/.orbit/) gets bootstrapped with config.toml
    assert!(home.join(".orbit").join("config.toml").exists());
    // Workspace root gets tasks/ dir
    assert!(workspace.join(".orbit").join("tasks").is_dir());
}

#[test]
fn config_show_rejects_legacy_watch_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[watch]\npersistence = { type = \"sqlite\", path = \"./.orbit/orbit.db\" }\n",
    )
    .expect("write config");

    orbit_in(dir.path())
        .args(["config", "show", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "watch config is no longer supported",
        ));
}

#[test]
fn top_level_help_omits_watch_command() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("watch").not());
}
