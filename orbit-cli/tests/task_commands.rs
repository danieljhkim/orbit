use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    let orbit_bin = assert_cmd::cargo::cargo_bin!("orbit");
    let mut paths = vec![orbit_bin.parent().expect("binary parent").to_path_buf()];
    if let Some(existing_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing_path));
    }
    let path = std::env::join_paths(paths).expect("joined PATH");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    // Prevent find_git_repo_root() from walking up to the real repo's .git
    // and writing task artifacts into the project's .orbit/ directory.
    cmd.env("ORBIT_ROOT", dir.join(".orbit"));
    cmd.env("PATH", path);
    cmd
}

fn add_task(dir: &Path, title: &str) -> String {
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
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_task_with_comment(dir: &Path, title: &str, comment: &str) -> String {
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
            "--comment",
            comment,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_agent_task(dir: &Path, title: &str) -> String {
    let output = orbit_in(dir)
        .env("ORBIT_TASK_ACTOR_KIND", "agent")
        .args([
            "task",
            "add",
            "--title",
            title,
            "--description",
            "test description",
            "--plan",
            "test plan",
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
fn task_add_json_returns_task_object() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "json add task",
            "--description",
            "json description",
            "--plan",
            "json plan",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["title"], "json add task");
    assert_eq!(task["description"], "json description");
    assert_eq!(task["plan"], "json plan");
    assert!(task["complexity"].is_null());
}

#[test]
fn task_add_json_includes_parent_id_when_provided() {
    let dir = tempfile::tempdir().expect("tempdir");
    let parent_id = add_task(dir.path(), "parent task");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "child task",
            "--description",
            "json description",
            "--plan",
            "json plan",
            "--parent",
            &parent_id,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["parent_id"], parent_id);
}

#[test]
fn task_add_json_includes_complexity_when_provided() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "complex task",
            "--description",
            "json description",
            "--plan",
            "json plan",
            "--complexity",
            "hard",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["complexity"], "hard");
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
fn task_add_comment_is_persisted_in_show_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task_with_comment(dir.path(), "commented task", "initial context");

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["comments"].as_array().expect("comments").len(), 1);
    assert_eq!(show["comments"][0]["by"], "human");
    assert_eq!(show["comments"][0]["message"], "initial context");
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
fn task_list_parent_filters_subtasks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let parent_id = add_task(dir.path(), "parent task");
    let child_output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "child task",
            "--description",
            "desc",
            "--plan",
            "plan",
            "--parent",
            &parent_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let child_id = String::from_utf8(child_output)
        .expect("utf8")
        .trim()
        .to_string();
    let _unrelated_id = add_task(dir.path(), "unrelated task");

    let output = orbit_in(dir.path())
        .args(["task", "list", "--all", "--parent", &parent_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], child_id);
    assert_eq!(arr[0]["parent_id"], parent_id);
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
fn task_show_displays_parent_task_when_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let parent_id = add_task(dir.path(), "parent task");
    let child_output = orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "child task",
            "--description",
            "desc",
            "--plan",
            "plan",
            "--parent",
            &parent_id,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let child_id = String::from_utf8(child_output)
        .expect("utf8")
        .trim()
        .to_string();

    orbit_in(dir.path())
        .args(["task", "show", &child_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Parent Task:"))
        .stdout(predicate::str::contains(&parent_id));
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
fn task_add_warns_when_parent_is_missing() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "task",
            "add",
            "--title",
            "child task",
            "--description",
            "desc",
            "--plan",
            "plan",
            "--parent",
            "T20260320-999999",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: parent task 'T20260320-999999' was not found",
        ));
}

#[test]
fn task_update_title_renames_task_and_records_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "original title");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--title", "renamed title"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task"));

    // Title must be visible in show output.
    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["title"], "renamed title");

    // History must contain a "renamed" event in the task YAML.
    let task_yaml = locate_task_yaml(dir.path(), &id);
    assert!(
        task_yaml.contains("renamed"),
        "task history must record a 'renamed' event; got:\n{task_yaml}"
    );
}

/// Walk .orbit/tasks/**/{id}/task.yaml and return its contents.
fn locate_task_yaml(root: &Path, id: &str) -> String {
    let tasks_root = root.join(".orbit").join("tasks");
    for status_dir in std::fs::read_dir(&tasks_root)
        .expect("read tasks dir")
        .flatten()
    {
        let candidate = status_dir.path().join(id).join("task.yaml");
        if candidate.exists() {
            return std::fs::read_to_string(&candidate).expect("read task yaml");
        }
    }
    panic!(
        "task.yaml not found for id={id} under {}",
        tasks_root.display()
    );
}

#[test]
fn task_update_updates_description_and_plan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "body-update");

    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--description",
            "updated description",
            "--plan",
            "updated plan",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["description"], "updated description");
    assert_eq!(show["plan"], "updated plan");
}

#[test]
fn task_update_json_returns_task_object() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "json update");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--title",
            "json updated title",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["id"], id);
    assert_eq!(task["title"], "json updated title");
}

#[test]
fn task_update_accepts_in_progress_status_alias() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "status alias");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in_progress"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["status"], "in-progress");
}

#[test]
fn task_show_and_list_use_cli_status_spelling() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "status display");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();

    orbit_in(dir.path())
        .args(["task", "show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status:"))
        .stdout(predicate::str::contains("in-progress"));

    orbit_in(dir.path())
        .args(["task", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));
}

#[test]
fn task_update_accepts_instructions_alias_for_plan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "instructions-update");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--instructions", "updated via alias"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["plan"], "updated via alias");
}

#[test]
fn task_update_comment_appends_without_replacing_existing_comments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task_with_comment(dir.path(), "comment append", "created");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--comment", "follow-up"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    let comments = show["comments"].as_array().expect("comments");
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["message"], "created");
    assert_eq!(comments[1]["by"], "human");
    assert_eq!(comments[1]["message"], "follow-up");
}

#[test]
fn task_direct_cli_uses_human_for_created_by_and_comment_author() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[user]\nname = \"daniel\"\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "configured actor");

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["created_by"], "human");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--comment", "follow-up"])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    let comments = show["comments"].as_array().expect("comments");
    assert_eq!(comments[0]["by"], "human");
    assert_eq!(comments[0]["message"], "follow-up");
}

#[test]
fn task_show_json_includes_agent_and_model_when_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "agent metadata");
    let task_yaml_path = task_dir(dir.path(), &id).join("task.yaml");
    let task_yaml = std::fs::read_to_string(&task_yaml_path).expect("read yaml");
    let mut saw_actor_identity = false;
    let rewritten = task_yaml
        .lines()
        .map(|line| {
            if line.starts_with("actor_identity:") {
                saw_actor_identity = true;
                "actor_identity: codex / gpt-5.4".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        saw_actor_identity,
        "task yaml should contain actor_identity field"
    );
    std::fs::write(&task_yaml_path, format!("{rewritten}\n")).expect("write yaml");

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["agent"], "codex");
    assert_eq!(show["model"], "gpt-5.4");
}

#[test]
fn task_update_rejects_blank_comment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "blank comment");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--comment", "   "])
        .assert()
        .failure()
        .stderr(predicate::str::contains("task comment must not be empty"));
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
fn task_archive_unarchive_and_delete_support_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "json lifecycle");

    let archive_output = orbit_in(dir.path())
        .args(["task", "archive", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let archived: serde_json::Value =
        serde_json::from_slice(&archive_output).expect("archive json");
    assert_eq!(archived["id"], id);
    assert_eq!(archived["status"], "archived");

    let unarchive_output = orbit_in(dir.path())
        .args(["task", "unarchive", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unarchived: serde_json::Value =
        serde_json::from_slice(&unarchive_output).expect("unarchive json");
    assert_eq!(unarchived["id"], id);
    assert_eq!(unarchived["status"], "backlog");

    let delete_output = orbit_in(dir.path())
        .args(["task", "delete", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let deleted: serde_json::Value = serde_json::from_slice(&delete_output).expect("delete json");
    assert_eq!(deleted["id"], id);
    assert_eq!(deleted["deleted"], true);
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
fn task_start_backlog_moves_to_in_progress() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "start backlog");

    orbit_in(dir.path())
        .args([
            "task",
            "start",
            &id,
            "--note",
            "picked up for implementation",
            "--comment",
            "starting now",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Started task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["status"], "in-progress");
    assert_eq!(show["assigned_to"], "human");
    assert_eq!(show["comments"][0]["message"], "starting now");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["event"], "started");
    assert_eq!(
        history.last().expect("latest")["note"],
        "picked up for implementation"
    );
    assert_eq!(history.last().expect("latest")["from_status"], "backlog");
    assert_eq!(history.last().expect("latest")["to_status"], "in_progress");
}

#[test]
fn task_start_with_explicit_identity_updates_provenance_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "precise provenance");
    let input = format!(
        "{{\"id\":\"{id}\",\"note\":\"picked up with explicit identity\",\"comment\":\"starting with provenance\",\"agent\":\"codex\",\"model\":\"gpt-5.4\"}}"
    );

    let output = orbit_in(dir.path())
        .args(["tool", "run", "orbit.task.start", "--input", &input])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");

    assert_eq!(task["status"], "in-progress");
    assert_eq!(task["assigned_to"], "codex / gpt-5.4");
    assert_eq!(task["agent"], "codex");
    assert_eq!(task["model"], "gpt-5.4");
    assert_eq!(task["comments"][0]["by"], "codex / gpt-5.4");
    assert_eq!(task["comments"][0]["message"], "starting with provenance");
    assert_eq!(
        task["history"]
            .as_array()
            .expect("history")
            .last()
            .expect("latest")["by"],
        "codex / gpt-5.4"
    );
}

#[test]
fn direct_task_start_with_explicit_identity_updates_provenance_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "direct precise provenance");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "start",
            &id,
            "--note",
            "picked up with explicit identity",
            "--comment",
            "starting with provenance",
            "--agent",
            "codex",
            "--model",
            "gpt-5.4",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");

    assert_eq!(task["status"], "in-progress");
    assert_eq!(task["assigned_to"], "codex / gpt-5.4");
    assert_eq!(task["agent"], "codex");
    assert_eq!(task["model"], "gpt-5.4");
    assert_eq!(task["comments"][0]["by"], "codex / gpt-5.4");
    assert_eq!(task["comments"][0]["message"], "starting with provenance");
    assert_eq!(
        task["history"]
            .as_array()
            .expect("history")
            .last()
            .expect("latest")["by"],
        "codex / gpt-5.4"
    );
}

#[test]
fn task_start_proposed_records_approval_and_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "start proposed");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "start",
            &id,
            "--note",
            "approved to begin immediately",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["status"], "in-progress");
    let history = task["history"].as_array().expect("history");
    assert!(
        history.iter().any(|entry| {
            entry["event"] == "proposal_approved"
                && entry["note"] == "approved to begin immediately"
                && entry["from_status"] == "proposed"
                && entry["to_status"] == "backlog"
        }),
        "proposal approval must remain visible in history"
    );
    assert_eq!(history.last().expect("latest")["event"], "started");
    assert_eq!(history.last().expect("latest")["from_status"], "proposed");
    assert_eq!(history.last().expect("latest")["to_status"], "in_progress");
}

#[test]
fn task_update_to_review_records_transition_details() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "review transition");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "Implemented and validated the change",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["event"], "status_changed");
    assert_eq!(
        history.last().expect("latest")["from_status"],
        "in_progress"
    );
    assert_eq!(history.last().expect("latest")["to_status"], "review");
}

#[test]
fn task_start_rejects_review_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "start review");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "Implemented initial change set",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args(["task", "start", &id])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "start requires 'proposed', 'backlog', 'someday', or 'blocked'",
        ));
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
    let id = add_agent_task(dir.path(), "approvable");

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
    assert_eq!(show["status"], "backlog");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["by"], "human");
    assert_eq!(
        history.last().expect("latest")["event"],
        "proposal_approved"
    );
    assert_eq!(
        history.last().expect("latest")["note"],
        "approved verbally in sync"
    );
    assert_eq!(history.last().expect("latest")["from_status"], "proposed");
    assert_eq!(history.last().expect("latest")["to_status"], "backlog");
}

#[test]
fn task_approve_json_returns_updated_task() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "approve json");

    let output = orbit_in(dir.path())
        .args(["task", "approve", &id, "--note", "json approved", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["id"], id);
    assert_eq!(task["status"], "backlog");
    assert_eq!(
        task["history"]
            .as_array()
            .expect("history")
            .last()
            .expect("latest")["event"],
        "proposal_approved"
    );
}

#[test]
fn task_approve_defaults_to_configured_user_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n[user]\nname = \"daniel\"\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "approve defaults");

    orbit_in(dir.path())
        .args(["task", "approve", &id, "--note", "config default approver"])
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
    assert_eq!(
        show["history"]
            .as_array()
            .expect("history")
            .last()
            .expect("latest")["by"],
        "human"
    );
}

#[test]
fn task_approve_comment_appends_with_approver_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "approvable comment");

    orbit_in(dir.path())
        .args([
            "task",
            "approve",
            &id,
            "--note",
            "approved verbally in sync",
            "--comment",
            "ready to schedule",
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
    assert_eq!(show["comments"][0]["by"], "human");
    assert_eq!(show["comments"][0]["message"], "ready to schedule");
}

#[test]
fn task_approve_review_to_done_records_transition_details() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "review approval");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "Ready for approval",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args(["task", "approve", &id, "--note", "looks good"])
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
    assert_eq!(show["status"], "done");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["event"], "review_approved");
    assert_eq!(history.last().expect("latest")["note"], "looks good");
    assert_eq!(history.last().expect("latest")["from_status"], "review");
    assert_eq!(history.last().expect("latest")["to_status"], "done");
}

#[test]
fn task_reject_proposed_to_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "rejectable");

    orbit_in(dir.path())
        .args([
            "task",
            "reject",
            &id,
            "--note",
            "Duplicate of an existing task",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rejected task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["status"], "rejected");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["by"], "human");
    assert_eq!(
        history.last().expect("latest")["event"],
        "proposal_rejected"
    );
    assert_eq!(
        history.last().expect("latest")["note"],
        "Duplicate of an existing task"
    );
}

#[test]
fn task_reject_json_returns_updated_task() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "reject json");

    let output = orbit_in(dir.path())
        .args(["task", "reject", &id, "--note", "json rejected", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: serde_json::Value = serde_json::from_slice(&output).expect("task json");
    assert_eq!(task["id"], id);
    assert_eq!(task["status"], "rejected");
    assert_eq!(
        task["history"]
            .as_array()
            .expect("history")
            .last()
            .expect("latest")["event"],
        "proposal_rejected"
    );
}

#[test]
fn task_reject_review_to_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "review reject");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "Implemented initial change set",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args([
            "task",
            "reject",
            &id,
            "--note",
            "Needs stronger coverage before merge",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rejected task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["status"], "rejected");
    let history = show["history"].as_array().expect("history");
    assert_eq!(history.last().expect("latest")["by"], "human");
    assert_eq!(history.last().expect("latest")["event"], "review_rejected");
    assert_eq!(
        history.last().expect("latest")["note"],
        "Needs stronger coverage before merge"
    );
    assert_eq!(history.last().expect("latest")["from_status"], "review");
    assert_eq!(history.last().expect("latest")["to_status"], "rejected");
}

#[test]
fn task_reject_comment_appends_with_reviewer_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "review reject with comment");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "Implemented initial change set",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args([
            "task",
            "reject",
            &id,
            "--note",
            "Needs stronger coverage before merge",
            "--comment",
            "add a regression test for comment ordering",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rejected task"));

    let show_output = orbit_in(dir.path())
        .args(["task", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: serde_json::Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["comments"][0]["by"], "human");
    assert_eq!(
        show["comments"][0]["message"],
        "add a regression test for comment ordering"
    );
}

#[test]
fn task_reject_requires_note() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "missing note");

    orbit_in(dir.path())
        .args(["task", "reject", &id])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ));
}

#[test]
fn task_list_ops_returns_signal_tier_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "ops test task");

    let output = orbit_in(dir.path())
        .args(["task", "list", "--ops"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    let tasks = tasks.as_array().expect("array");
    assert!(!tasks.is_empty());

    let task = tasks.iter().find(|t| t["id"] == id).expect("task in list");

    // Required signal fields present.
    assert!(task.get("id").is_some());
    assert!(task.get("title").is_some());
    assert!(task.get("type").is_some());
    assert!(task.get("status").is_some());
    assert!(task.get("priority").is_some());

    // Verbose fields must be absent.
    assert!(task.get("description").is_none());
    assert!(task.get("plan").is_none());
    assert!(task.get("execution_summary").is_none());
    assert!(task.get("context_files").is_none());
    assert!(task.get("comments").is_none());
}

#[test]
fn task_reject_proposed_moves_to_rejected_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "rejected-dir-test");

    orbit_in(dir.path())
        .args(["task", "reject", &id, "--note", "invalid scope"])
        .assert()
        .success();

    // File must be under rejected/ on disk.
    let home = dir.path().join(".orbit");
    let rejected_dir = home.join("tasks").join("rejected").join(&id);
    assert!(rejected_dir.exists(), "task dir must be under rejected/");
    // Must not remain under proposed/.
    let proposed_dir = home.join("tasks").join("proposed").join(&id);
    assert!(
        !proposed_dir.exists(),
        "task dir must not remain in proposed/"
    );
}

#[test]
fn task_reject_review_moves_to_rejected_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");
    let id = add_task(dir.path(), "review-rejected-dir-test");

    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "in-progress"])
        .assert()
        .success();
    orbit_in(dir.path())
        .args([
            "task",
            "update",
            &id,
            "--status",
            "review",
            "--execution-summary",
            "initial implementation done",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args(["task", "reject", &id, "--note", "missing coverage"])
        .assert()
        .success();

    let home = dir.path().join(".orbit");
    let rejected_dir = home.join("tasks").join("rejected").join(&id);
    assert!(rejected_dir.exists(), "task dir must be under rejected/");
    let review_dir = home.join("tasks").join("review").join(&id);
    assert!(!review_dir.exists(), "task dir must not remain in review/");
}

#[test]
fn task_list_filtered_by_rejected_shows_rejected_tasks() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");
    let id = add_agent_task(dir.path(), "filterable-rejected");

    orbit_in(dir.path())
        .args(["task", "reject", &id, "--note", "out of scope"])
        .assert()
        .success();

    let output = orbit_in(dir.path())
        .args(["task", "list", "--status", "rejected", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");
    assert!(
        tasks.iter().any(|t| t["id"] == id),
        "rejected task must appear in filtered list"
    );
    assert!(
        tasks.iter().all(|t| t["status"] == "rejected"),
        "all tasks must be rejected"
    );
}

#[test]
fn task_list_default_shows_only_active_work() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let backlog_id = add_task(dir.path(), "backlog-task");
    let proposed_id = add_agent_task(dir.path(), "proposed-task");

    let output = orbit_in(dir.path())
        .args(["task", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");

    assert!(
        tasks.iter().any(|t| t["id"] == backlog_id),
        "backlog task must appear in default list"
    );
    assert!(
        !tasks.iter().any(|t| t["id"] == proposed_id),
        "proposed task must not appear in default list"
    );
}

#[test]
fn task_list_all_shows_every_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let backlog_id = add_task(dir.path(), "backlog-task-all");
    let proposed_id = add_agent_task(dir.path(), "proposed-task-all");

    let output = orbit_in(dir.path())
        .args(["task", "list", "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");

    assert!(
        tasks.iter().any(|t| t["id"] == backlog_id),
        "backlog task must appear with --all"
    );
    assert!(
        tasks.iter().any(|t| t["id"] == proposed_id),
        "proposed task must appear with --all"
    );
}

#[test]
fn task_list_multi_status_filter() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let backlog_id = add_task(dir.path(), "multi-backlog");
    let proposed_id = add_agent_task(dir.path(), "multi-proposed");

    let output = orbit_in(dir.path())
        .args(["task", "list", "--status", "backlog,proposed", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");

    assert!(
        tasks.iter().any(|t| t["id"] == backlog_id),
        "backlog task must appear with --status backlog,proposed"
    );
    assert!(
        tasks.iter().any(|t| t["id"] == proposed_id),
        "proposed task must appear with --status backlog,proposed"
    );
}

#[test]
fn task_approve_multi_id_approves_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let id1 = add_agent_task(dir.path(), "bulk-approve-1");
    let id2 = add_agent_task(dir.path(), "bulk-approve-2");

    let output = orbit_in(dir.path())
        .args(["task", "approve", &id1, &id2, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t["id"] == id1));
    assert!(tasks.iter().any(|t| t["id"] == id2));
    assert!(tasks.iter().all(|t| t["status"] == "backlog"));
}

#[test]
fn task_reject_multi_id_rejects_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let id1 = add_agent_task(dir.path(), "bulk-reject-1");
    let id2 = add_agent_task(dir.path(), "bulk-reject-2");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "reject",
            &id1,
            &id2,
            "--note",
            "out of scope",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t["id"] == id1));
    assert!(tasks.iter().any(|t| t["id"] == id2));
    assert!(tasks.iter().all(|t| t["status"] == "rejected"));
}

#[test]
fn task_approve_all_proposed_with_yes_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let id1 = add_agent_task(dir.path(), "all-proposed-1");
    let id2 = add_agent_task(dir.path(), "all-proposed-2");

    let output = orbit_in(dir.path())
        .args(["task", "approve", "--all-proposed", "--yes", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t["id"] == id1));
    assert!(tasks.iter().any(|t| t["id"] == id2));
    assert!(tasks.iter().all(|t| t["status"] == "backlog"));
}

#[test]
fn task_reject_all_proposed_with_yes_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".orbit")).expect("create .orbit");
    std::fs::write(
        dir.path().join(".orbit").join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let id1 = add_agent_task(dir.path(), "all-reject-proposed-1");
    let id2 = add_agent_task(dir.path(), "all-reject-proposed-2");

    let output = orbit_in(dir.path())
        .args([
            "task",
            "reject",
            "--all-proposed",
            "--yes",
            "--note",
            "batch rejected",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let tasks: serde_json::Value = serde_json::from_slice(&output).expect("json");
    let tasks = tasks.as_array().expect("array");
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t["id"] == id1));
    assert!(tasks.iter().any(|t| t["id"] == id2));
    assert!(tasks.iter().all(|t| t["status"] == "rejected"));
}

#[test]
fn task_update_status_rejected_is_allowed_with_relaxed_rules() {
    let dir = tempfile::tempdir().expect("tempdir");
    let id = add_task(dir.path(), "direct-reject-allowed");

    // Relaxed transition rules allow setting status to rejected directly.
    orbit_in(dir.path())
        .args(["task", "update", &id, "--status", "rejected"])
        .assert()
        .success();
}
