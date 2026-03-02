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

fn add_task(dir: &Path, title: &str) -> String {
    let workspace = dir
        .canonicalize()
        .expect("canonical workspace")
        .to_string_lossy()
        .to_string();
    let output = orbit_in(dir)
        .args([
            "task",
            "add",
            "--title",
            title,
            "--description",
            "test description",
            "--instructions",
            "test instructions",
            "--workspace",
            &workspace,
            "--proposed-by",
            "test-user",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

#[test]
fn task_add_prints_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "test task");
    assert!(id.starts_with("T"), "id should start with T: {id}");
}

#[test]
fn task_list_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    orbit_in(dir.path())
        .args(["task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ID"));
}

#[test]
fn task_list_after_add() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "listed task");
    orbit_in(dir.path())
        .args(["task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&id))
        .stdout(predicate::str::contains("listed task"));
}

#[test]
fn task_list_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _id = add_task(dir.path(), "json task");
    let output = orbit_in(dir.path())
        .args(["task", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(parsed.is_array());
    let arr = parsed.as_array().expect("array");
    assert!(arr.iter().any(|t| t["title"] == "json task"));
}

#[test]
fn task_show_displays_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "showable task");
    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("ID:"))
        .stdout(predicate::str::contains("showable task"))
        .stdout(predicate::str::contains("Status:"));
}

#[test]
fn task_show_nonexistent() {
    let dir = tempfile::tempdir().expect("tempdir");
    orbit_in(dir.path())
        .args(["task", "show", "task-nonexistent"])
        .assert()
        .failure();
}

#[test]
fn task_update_rejects_non_updatable_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "non-updatable");
    orbit_in(dir.path())
        .args(["task", "update", &id, "--title", "ignored"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--title'"));
}

#[test]
fn task_archive_and_unarchive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "archivable");
    orbit_in(dir.path())
        .args(["task", "archive", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Archived"));

    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("archived"));

    orbit_in(dir.path())
        .args(["task", "unarchive", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unarchived"));

    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("backlog"));
}

#[test]
fn task_delete_removes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "deletable");
    orbit_in(dir.path())
        .args(["task", "delete", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted"));

    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .failure();
}

#[test]
fn task_search_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _id = add_task(dir.path(), "unique-searchable-xyz");
    orbit_in(dir.path())
        .args(["task", "search", "unique-searchable"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unique-searchable-xyz"));
}

#[test]
fn task_workspace_is_normalized_on_add() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("repo");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let workspace_canonical = workspace.canonicalize().expect("canonical workspace");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "workspace task",
            "--description",
            "workspace description",
            "--instructions",
            "workspace instructions",
            "--workspace",
            workspace.to_string_lossy().as_ref(),
            "--proposed-by",
            "workspace-proposer",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = String::from_utf8(output).expect("utf8").trim().to_string();

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(
        show["workspace_path"],
        workspace_canonical.to_string_lossy().to_string()
    );
}

#[test]
fn task_approve_proposed_to_backlog() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "approvable");

    // Task should start as proposed since approval is required
    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("proposed"));

    orbit_in(dir.path())
        .args([
            "task",
            "approve",
            &id,
            "--by",
            "daniel",
            "--note",
            "approved verbally in sync",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Approved task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["proposal_approved_by"], "daniel");
    assert_eq!(show["proposal_decision_note"], "approved verbally in sync");
    assert_eq!(show["status"], "backlog");
}
