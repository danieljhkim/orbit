#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn spoke_routes_only_self_registration_and_rejects_direct_administration() {
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
            "spoke",
            "--host-mode",
            "spoke",
        ])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "workspace",
            "init",
            "--name",
            "spoke-workspace",
            "--role",
            "replica",
            "--owner",
            "hm_owner",
        ])
        .assert()
        .success();

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "host",
            "register",
            "--machine-id",
            "hm_remote",
            "--host-id",
            "remote",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "reads machine_id and host_id only from validated host.toml",
        ));

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "register"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mcp.toml"));

    let mutation_commands = [
        vec!["host", "rename", "missing-host", "new-name"],
        vec!["host", "retire", "missing-host"],
        vec![
            "workspace",
            "link",
            "missing-workspace",
            "--owner",
            "missing-host",
        ],
    ];
    for args in mutation_commands {
        cargo_bin_cmd!("orbit")
            .current_dir(&work)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("ORBIT_ROOT")
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("hub-local in v1"))
            .stderr(predicate::str::contains("configured as mode 'spoke'"))
            .stderr(predicate::str::contains("unknown host").not())
            .stderr(predicate::str::contains("unknown workspace").not());
    }

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("hub-local in v1"))
        .stdout(predicate::str::contains("no hosts registered").not());
}

#[test]
fn explicit_local_machine_registration_uses_atomic_hub_bootstrap() {
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
            "hub",
            "--host-mode",
            "hub",
        ])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["workspace", "init", "--name", "hub-workspace"])
        .assert()
        .success();

    let host_toml =
        std::fs::read_to_string(home.join(".orbit/host.toml")).expect("read host identity");
    let host: toml::Value = toml::from_str(&host_toml).expect("parse host identity");
    let machine_id = host["machine_id"].as_str().expect("machine id");

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "host",
            "register",
            "--machine-id",
            machine_id,
            "--host-id",
            "hub",
        ])
        .assert()
        .success();

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(machine_id))
        .stdout(predicate::str::contains("yes"));

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "host",
            "register",
            "--machine-id",
            machine_id,
            "--host-id",
            "shadow-name",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "explicit declaration for this hub machine_id",
        ));
}

#[test]
fn rejected_current_host_rename_preserves_local_identity_and_registry_revision() {
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
            "hub",
            "--host-mode",
            "hub",
        ])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["workspace", "init", "--name", "hub-workspace"])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "register"])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args([
            "host",
            "register",
            "--machine-id",
            "hm_remote",
            "--host-id",
            "remote",
        ])
        .assert()
        .success();

    let identity_path = home.join(".orbit/host.toml");
    let before_identity = std::fs::read(&identity_path).expect("read host.toml");
    let before_host: toml::Value =
        toml::from_str(std::str::from_utf8(&before_identity).expect("host.toml must be UTF-8"))
            .expect("parse original host.toml");
    let machine_id = before_host["machine_id"]
        .as_str()
        .expect("original machine_id")
        .to_string();
    let db_path = home.join(".orbit/orbit.db");
    let revision = || {
        Connection::open(&db_path)
            .expect("open store")
            .query_row(
                "SELECT registry_revision FROM hub_registry_metadata WHERE id = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("read revision")
    };
    let before_revision = revision();

    for (new_name, expected_error) in [
        ("path\\name", "logical registry identifier"),
        ("remote", "already reserved"),
    ] {
        cargo_bin_cmd!("orbit")
            .current_dir(&work)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("ORBIT_ROOT")
            .args(["host", "rename", "hub", new_name])
            .assert()
            .failure()
            .stderr(predicate::str::contains(expected_error));
        assert_eq!(
            std::fs::read(&identity_path).expect("reread host.toml"),
            before_identity,
            "rejected name {new_name:?} changed host.toml"
        );
        assert_eq!(
            revision(),
            before_revision,
            "rejected name {new_name:?} advanced the registry revision"
        );
    }

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "rename", "hub", "hub-renamed"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "local host.toml and the hub registry now agree",
        ));
    let after_host: toml::Value =
        toml::from_str(&std::fs::read_to_string(&identity_path).expect("read renamed host.toml"))
            .expect("parse renamed host.toml");
    assert_eq!(after_host["machine_id"].as_str(), Some(machine_id.as_str()));
    assert_eq!(after_host["host_id"].as_str(), Some("hub-renamed"));
    assert_eq!(after_host["mode"].as_str(), Some("hub"));
    assert_eq!(
        revision(),
        before_revision + 1,
        "successful coordinated rename must advance the snapshot once"
    );

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hub-renamed"))
        .stdout(predicate::str::contains("remote"));

    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "retire", "remote"])
        .assert()
        .success();
    cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["host", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("remote"))
        .stdout(predicate::str::contains("retired"));
}
