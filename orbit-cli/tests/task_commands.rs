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
            "--plan",
            "test plan",
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

fn task_dir(dir: &Path, id: &str) -> std::path::PathBuf {
    let tasks_root = dir.join(".orbit").join("tasks");
    for status in [
        "proposed",
        "backlog",
        "in_progress",
        "review",
        "done",
        "blocked",
        "archived",
    ] {
        let candidate = tasks_root.join(status).join(id);
        if candidate.exists() {
            return candidate;
        }
    }
    tasks_root.join("missing").join(id)
}

#[test]
fn task_add_prints_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "test task");
    assert!(id.starts_with("T"), "id should start with T: {id}");
}

#[test]
fn task_add_creates_bundle_layout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "bundle task");
    let task_dir = task_dir(dir.path(), &id);

    assert!(task_dir.join("task.yaml").exists());
    assert!(task_dir.join("plan.md").exists());
    assert!(task_dir.join("execution-summary.md").exists());
    assert!(task_dir.join("artifacts").is_dir());

    let task_yaml = std::fs::read_to_string(task_dir.join("task.yaml")).expect("read task yaml");
    assert!(task_yaml.contains("schema_version: 4"));
    assert!(task_yaml.contains("description: test description"));
    assert!(!task_yaml.contains("plan:"));
    assert!(!task_yaml.contains("execution_summary:"));
    assert_eq!(
        std::fs::read_to_string(task_dir.join("plan.md")).expect("read plan"),
        "test plan"
    );
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
    assert!(arr.iter().any(|t| t["plan"] == "test plan"));
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
        .stdout(predicate::str::contains("Plan:"))
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
    let initial_dir = task_dir(dir.path(), &id);
    std::fs::write(
        initial_dir.join("artifacts").join("report.md"),
        "# execution report\n",
    )
    .expect("write artifact");
    orbit_in(dir.path())
        .args(["task", "archive", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Archived"));

    let archived_dir = dir
        .path()
        .join(".orbit")
        .join("tasks")
        .join("archived")
        .join(&id);
    assert!(!initial_dir.exists());
    assert_eq!(
        std::fs::read_to_string(archived_dir.join("artifacts").join("report.md"))
            .expect("artifact moved"),
        "# execution report\n"
    );

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

    let backlog_dir = dir
        .path()
        .join(".orbit")
        .join("tasks")
        .join("backlog")
        .join(&id);
    assert!(!archived_dir.exists());
    assert_eq!(
        std::fs::read_to_string(backlog_dir.join("artifacts").join("report.md"))
            .expect("artifact moved back"),
        "# execution report\n"
    );

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
    let task_dir = task_dir(dir.path(), &id);
    orbit_in(dir.path())
        .args(["task", "delete", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted"));

    assert!(!task_dir.exists());

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
            "workspace plan",
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
    assert_eq!(show["plan"], "workspace plan");
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
