#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::tempdir;

fn initialized_workspace() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&work).expect("create work");

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "init",
            "--non-interactive",
            "--host-name",
            "local",
            "--task-prefix",
            "DE",
        ])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["workspace", "init", "--name", "local-workspace"])
        .assert()
        .success();
    (temp, home, work)
}

#[test]
fn fleet_administration_and_inventory_are_absent_from_cli_and_tool_registry() {
    let (_temp, home, work) = initialized_workspace();

    for subcommand in ["register", "list", "retire"] {
        cargo_bin_cmd!("orbit")
            .current_dir(&work)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("ORBIT_ROOT")
            .args(["host", subcommand])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["workspace", "link"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["tool", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("orbit.host.").not())
        .stdout(predicate::str::contains("register-spoke").not());
}

#[test]
fn rename_updates_host_and_local_owner_names_without_changing_stable_identity() {
    let (_temp, home, work) = initialized_workspace();
    let identity_path = home.join(".orbit/host.toml");
    let registry_path = home.join(".orbit/workspaces.json");
    let before: toml::Value =
        toml::from_str(&std::fs::read_to_string(&identity_path).expect("read host identity"))
            .expect("parse host identity");
    let machine_id = before["machine_id"]
        .as_str()
        .expect("machine id")
        .to_string();
    assert_eq!(before["task_prefix"].as_str(), Some("DE"));

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "rename", "local", "renamed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 local workspace owner record"));

    let after: toml::Value =
        toml::from_str(&std::fs::read_to_string(&identity_path).expect("read renamed identity"))
            .expect("parse renamed identity");
    assert_eq!(after["machine_id"].as_str(), Some(machine_id.as_str()));
    assert_eq!(after["host_id"].as_str(), Some("renamed"));
    assert_eq!(after["task_prefix"].as_str(), Some("DE"));

    let registry: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&registry_path).expect("read workspace registry"),
    )
    .expect("parse workspace registry");
    assert_eq!(registry["owner_host_ids"][&machine_id], "renamed");
    assert_eq!(
        registry["workspaces"][0]["owner_machine_id"], machine_id,
        "rename must not change stable owner binding"
    );

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "rename", "local", "again"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "current host.toml names 'renamed'",
        ));
}
