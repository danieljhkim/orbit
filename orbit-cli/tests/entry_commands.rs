use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd
}

fn add_task(dir: &Path, title: &str) -> String {
    let output = orbit_in(dir)
        .args(["task", "add", "--title", title])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_task_entry(dir: &Path, task_id: &str, body: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "entry",
            "add",
            "--entity-type",
            "task",
            "--entity-id",
            task_id,
            "--entry-type",
            "comment",
            "--author-type",
            "human",
            "--author-id",
            "daniel",
            "--body",
            body,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

#[test]
fn entry_add_prints_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task(dir.path(), "entry");
    let entry_id = add_task_entry(dir.path(), &task_id, "hello");
    assert!(
        entry_id.starts_with("entry-"),
        "id should start with entry-: {entry_id}"
    );
}

#[test]
fn entry_add_json_output_is_valid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task(dir.path(), "entry");

    let output = orbit_in(dir.path())
        .args([
            "entry",
            "add",
            "--entity-type",
            "task",
            "--entity-id",
            &task_id,
            "--entry-type",
            "comment",
            "--author-type",
            "human",
            "--author-id",
            "daniel",
            "--body",
            "json-body",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert_eq!(parsed["entity_type"], "task");
    assert_eq!(parsed["entry_type"], "comment");
    assert_eq!(parsed["body"], "json-body");
}

#[test]
fn entry_list_json_is_deterministic_by_sequence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task(dir.path(), "entry");
    let _ = add_task_entry(dir.path(), &task_id, "first");
    let _ = add_task_entry(dir.path(), &task_id, "second");

    let output = orbit_in(dir.path())
        .args([
            "entry",
            "list",
            "--entity-type",
            "task",
            "--entity-id",
            &task_id,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["sequence_number"], 1);
    assert_eq!(arr[1]["sequence_number"], 2);
    assert_eq!(arr[0]["body"], "first");
    assert_eq!(arr[1]["body"], "second");
}

#[test]
fn entry_add_rejects_workflow_entity_type_in_v1() {
    let dir = tempfile::tempdir().expect("tempdir");
    orbit_in(dir.path())
        .args([
            "entry",
            "add",
            "--entity-type",
            "workflow",
            "--entity-id",
            "workflow-1",
            "--entry-type",
            "comment",
            "--author-type",
            "human",
            "--author-id",
            "daniel",
            "--body",
            "note",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported entity type in v1: workflow",
        ));
}
