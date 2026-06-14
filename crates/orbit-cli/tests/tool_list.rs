#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::tempdir;

const INACTIVE_TOOL_NAMES: &[&str] = &[
    "orbit.docs.index",
    "orbit.docs.migrate",
    "orbit.docs.add",
    "orbit.docs.list",
    "orbit.docs.show",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.semantic.index",
    "orbit.semantic.install",
    "orbit.semantic.stats",
    "orbit.learning.sync",
    "orbit.learning.list",
    "orbit.friction.stats",
];

#[test]
fn tool_list_all_shows_inactive_lock_reservation_with_required_input_shape() {
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
        .args(["tool", "list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("orbit.task.locks.reserve"))
        .stdout(predicate::str::contains("STATUS"))
        .stdout(predicate::str::contains("inactive"))
        .stdout(predicate::str::contains(
            "Exactly one of `task_ids` or `files`",
        ));
}

#[test]
fn tool_list_json_hides_inactive_tools_by_default() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&work).expect("create work");

    let output = cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["tool", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tools: Vec<serde_json::Value> = serde_json::from_slice(&output).expect("tool list JSON");
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name").to_string())
        .collect::<BTreeSet<_>>();

    for inactive in INACTIVE_TOOL_NAMES {
        assert!(
            !names.contains(*inactive),
            "inactive tool must not be visible through default `orbit tool list`: {inactive}"
        );
    }
}

#[test]
fn tool_list_json_all_includes_inactive_tools_with_status() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&work).expect("create work");

    let output = cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["tool", "list", "--json", "--all"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tools: Vec<serde_json::Value> = serde_json::from_slice(&output).expect("tool list JSON");

    for inactive in INACTIVE_TOOL_NAMES {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == *inactive)
            .unwrap_or_else(|| panic!("inactive tool missing from --all: {inactive}"));
        assert_eq!(tool["status"], "inactive");
        assert_eq!(tool["active"], false);
    }
}

#[test]
fn tool_list_json_all_includes_parameter_schema_for_inactive_tools() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&work).expect("create work");

    let output = cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["tool", "list", "--json", "--all"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tools: Vec<serde_json::Value> = serde_json::from_slice(&output).expect("tool list JSON");
    let reserve = tools
        .iter()
        .find(|tool| tool["name"] == "orbit.task.locks.reserve")
        .expect("reserve tool");
    let parameters = reserve["parameters"].as_array().expect("parameters array");
    assert!(parameters.iter().any(|param| {
        param["name"] == "task_ids"
            && param["param_type"] == "string_list"
            && param["required"] == false
    }));
    assert!(parameters.iter().any(|param| {
        param["name"] == "files"
            && param["param_type"] == "string_list"
            && param["required"] == false
    }));
}

#[test]
fn tool_list_json_includes_task_show_context_parameters() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&work).expect("create work");

    let output = cargo_bin_cmd!("orbit")
        .current_dir(&work)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("ORBIT_ROOT")
        .args(["tool", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tools: Vec<serde_json::Value> = serde_json::from_slice(&output).expect("tool list JSON");
    let task_show = tools
        .iter()
        .find(|tool| tool["name"] == "orbit.task.show")
        .expect("task show tool");
    let parameters = task_show["parameters"]
        .as_array()
        .expect("parameters array");
    assert!(parameters.iter().any(|param| {
        param["name"] == "with_context"
            && param["param_type"] == "boolean"
            && param["required"] == false
    }));
    assert!(parameters.iter().any(|param| {
        param["name"] == "max_docs"
            && param["param_type"] == "integer"
            && param["required"] == false
    }));
}

#[test]
fn tool_show_displays_lock_reservation_shapes() {
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
        .args(["tool", "show", "orbit.task.locks.reserve"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status:"))
        .stdout(predicate::str::contains("inactive"))
        .stdout(predicate::str::contains("task_ids"))
        .stdout(predicate::str::contains("files"))
        .stdout(predicate::str::contains("optional"))
        .stdout(predicate::str::contains(
            "Exactly one of `task_ids` or `files`",
        ));
}

#[test]
fn tool_run_rejects_inactive_tools() {
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
            "tool",
            "run",
            "orbit.learning.list",
            "--input",
            "{\"model\":\"codex\"}",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("inactive"));
}
