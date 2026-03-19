use assert_cmd::Command;
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

fn write_skill(dir: &Path, id: &str) {
    let skill_dir = dir.join(".orbit").join("skills").join(id);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!(
            "---\nname: {id}\ndescription: Test skill.\n---\n\n# {id}\n\n## Purpose\nTest skill.\n\n## Behavioral Constraints\n- deterministic\n\n## Output Requirements\n- json\n"
        ),
    )
    .expect("write skill");
}

fn init_orbit(dir: &Path) {
    orbit_in(dir).args(["init"]).assert().success();
}

fn add_task(dir: &Path, title: &str) -> Value {
    let output = orbit_in(dir)
        .args([
            "task",
            "add",
            "--title",
            title,
            "--description",
            "task description",
            "--plan",
            "task plan",
            "--workspace",
            &dir.to_string_lossy(),
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("task json")
}

fn show_task(dir: &Path, id: &str) -> Value {
    let output = orbit_in(dir)
        .args(["task", "show", id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("task show json")
}

fn add_job(dir: &Path, target_id: &str, agent_cli: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--target-id",
            target_id,
            "--agent-cli",
            agent_cli,
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<Value>(&output).expect("job json")["job_id"]
        .as_str()
        .expect("job id")
        .to_string()
}

#[test]
fn activity_add_show_list_delete_json_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(dir.path(), "orbit-assess-codebase");
    write_skill(dir.path(), "execution-audit");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-cli-1",
            "--type",
            "agent_invoke",
            "--description",
            "CLI activity test",
            "--input-schema",
            "{\"type\":\"object\"}",
            "--output-schema",
            "{\"type\":\"object\"}",
            "--spec-config",
            "{\"instruction\":\"Inspect repository metrics and summarize them.\",\"skill_refs\":[\"orbit-assess-codebase\",\"execution-audit\"]}",
            "--json",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-cli-1", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["id"], "spec-cli-1");
    assert_eq!(show["type"], "agent_invoke");
    assert_eq!(
        show["spec_config"]["instruction"],
        "Inspect repository metrics and summarize them."
    );
    assert_eq!(
        show["spec_config"]["skill_refs"][0],
        "orbit-assess-codebase"
    );
    assert_eq!(show["is_active"], true);

    let list_output = orbit_in(dir.path())
        .args(["activity", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    assert!(
        list.as_array()
            .expect("array")
            .iter()
            .any(|spec| spec["id"] == "spec-cli-1")
    );

    orbit_in(dir.path())
        .args(["activity", "delete", "spec-cli-1"])
        .assert()
        .success();

    let list_after_delete = orbit_in(dir.path())
        .args(["activity", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list_after_delete: Value =
        serde_json::from_slice(&list_after_delete).expect("list json after delete");
    assert!(
        !list_after_delete
            .as_array()
            .expect("array")
            .iter()
            .any(|spec| spec["id"] == "spec-cli-1")
    );
}

#[test]
fn activity_delete_json_returns_deleted_true() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-delete-json",
            "--description",
            "delete json test",
            "--json",
        ])
        .assert()
        .success();

    let output = orbit_in(dir.path())
        .args(["activity", "delete", "spec-delete-json", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let deleted: Value = serde_json::from_slice(&output).expect("delete json");
    assert_eq!(deleted["id"], "spec-delete-json");
    assert_eq!(deleted["deleted"], true);
}

#[test]
fn activity_add_defaults_type_and_schemas_when_omitted() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-cli-defaults",
            "--description",
            "CLI activity defaults test",
            "--json",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-cli-defaults", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["id"], "spec-cli-defaults");
    assert_eq!(show["type"], "agent_invoke");
    assert_eq!(show["spec_config"], serde_json::json!({}));
    assert_eq!(show["input_schema_json"], serde_json::json!({}));
    assert_eq!(show["output_schema_json"], serde_json::json!({}));
}

#[test]
fn activity_run_executes_without_creating_a_job() {
    let dir = tempfile::tempdir().expect("tempdir");
    let args_capture = dir.path().join("activity-args.txt");
    let stdin_capture = dir.path().join("activity-stdin.json");
    let script_path = dir.path().join("mock-agent");
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nprintf '%s' \"$@\" > \"{args}\"\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":4}}'\n",
            args = args_capture.to_string_lossy(),
            stdin = stdin_capture.to_string_lossy(),
        ),
    )
    .expect("write mock agent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod mock agent");
    }

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-cli-run",
            "--description",
            "CLI activity run test",
            "--json",
        ])
        .assert()
        .success();

    let run_output = orbit_in(dir.path())
        .args([
            "activity",
            "run",
            "spec-cli-run",
            "--agent-cli",
            &script_path.to_string_lossy(),
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["activity_id"], "spec-cli-run");
    assert_eq!(run["state"], "success");

    let args_raw = std::fs::read_to_string(args_capture).expect("args capture");
    assert!(args_raw.contains("activity"));
    assert!(!args_raw.contains("--job-id"));

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    assert!(stdin_raw.contains("\"activity\""));
    assert!(!stdin_raw.contains("\"job\""));
}

#[test]
fn cli_command_activity_run_does_not_require_agent_cli() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("emit-json.sh");
    std::fs::write(
        &script_path,
        "#!/bin/sh\nprintf '{\"exit_code\":0}' > \"$ORBIT_OUTPUT_FILE\"\n",
    )
    .expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
    }

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-cli-command-run",
            "--spec-type",
            "cli_command",
            "--description",
            "CLI command activity run test",
            "--output-schema",
            "{\"type\":\"object\",\"properties\":{\"exit_code\":{\"type\":\"integer\"}},\"required\":[\"exit_code\"]}",
            "--spec-config",
            &format!(
                "{{\"command\":\"{}\",\"expected_exit_codes\":[0]}}",
                script_path.to_string_lossy()
            ),
            "--json",
        ])
        .assert()
        .success();

    let run_output = orbit_in(dir.path())
        .args(["activity", "run", "spec-cli-command-run", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["activity_id"], "spec-cli-command-run");
    assert_eq!(run["state"], "success");
}

#[test]
fn activity_update_description_changes_field() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-update-desc",
            "--description",
            "original description",
            "--json",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args([
            "activity",
            "update",
            "spec-update-desc",
            "--description",
            "updated description",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-update-desc", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["description"], "updated description");
}

#[test]
fn activity_update_returns_json_when_flag_set() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-update-json",
            "--description",
            "before update",
            "--json",
        ])
        .assert()
        .success();

    let update_output = orbit_in(dir.path())
        .args([
            "activity",
            "update",
            "spec-update-json",
            "--description",
            "after update",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let updated: Value = serde_json::from_slice(&update_output).expect("update json");
    assert_eq!(updated["id"], "spec-update-json");
    assert_eq!(updated["description"], "after update");
}

#[test]
fn activity_update_spec_config_replaces_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(dir.path(), "skill-a");
    write_skill(dir.path(), "skill-b");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-update-skills",
            "--description",
            "activity with skills",
            "--spec-config",
            "{\"skill_refs\":[\"skill-a\"]}",
            "--json",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args([
            "activity",
            "update",
            "spec-update-skills",
            "--spec-config",
            "{\"skill_refs\":[\"skill-b\"]}",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-update-skills", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    let refs = show["spec_config"]["skill_refs"].as_array().expect("array");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0], "skill-b");
}

#[test]
fn activity_update_unknown_id_fails() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "update",
            "nonexistent-activity",
            "--description",
            "whatever",
        ])
        .assert()
        .failure();
}

#[test]
fn legacy_workflow_command_is_not_supported() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["workflow", "list"])
        .assert()
        .failure();
}

#[test]
fn activity_list_ops_returns_signal_tier_json() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path()).args(["init"]).assert().success();

    let output = orbit_in(dir.path())
        .args(["activity", "list", "--ops"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let activities: Value = serde_json::from_slice(&output).expect("valid json");
    let activities = activities.as_array().expect("array");
    assert!(!activities.is_empty());

    let activity = &activities[0];

    // Required signal fields present.
    assert!(activity.get("id").is_some());
    assert!(activity.get("type").is_some());
    assert!(activity.get("description").is_some());
    assert!(activity.get("is_active").is_some());

    // Verbose fields must be absent.
    assert!(activity.get("spec_config").is_none());
    assert!(activity.get("input_schema_json").is_none());
    assert!(activity.get("output_schema_json").is_none());
}

#[test]
fn activity_update_inactive_deactivates_activity() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-deactivate",
            "--description",
            "active by default",
            "--json",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args(["activity", "update", "spec-deactivate", "--inactive"])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-deactivate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["is_active"], false);
}

#[test]
fn activity_update_active_reactivates_inactive_activity() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-reactivate",
            "--description",
            "will be toggled",
            "--json",
        ])
        .assert()
        .success();

    // Deactivate first.
    orbit_in(dir.path())
        .args(["activity", "update", "spec-reactivate", "--inactive"])
        .assert()
        .success();

    // Now reactivate.
    orbit_in(dir.path())
        .args(["activity", "update", "spec-reactivate", "--active"])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-reactivate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["is_active"], true);
}

#[test]
fn update_task_activity_is_seeded_on_init() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_orbit(dir.path());

    let output = orbit_in(dir.path())
        .args(["activity", "show", "update_task", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let activity: Value = serde_json::from_slice(&output).expect("activity json");
    assert_eq!(activity["id"], "update_task");
    assert_eq!(activity["type"], "automation");
    assert_eq!(activity["spec_config"]["action"], "update_task");
}

#[test]
fn update_task_activity_job_run_updates_task_and_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_orbit(dir.path());

    let task = add_task(dir.path(), "Ship update_task");
    let task_id = task["id"].as_str().expect("task id").to_string();

    orbit_in(dir.path())
        .args(["task", "start", &task_id, "--json"])
        .assert()
        .success();

    let job_id = add_job(dir.path(), "update_task", "");
    let run_output = orbit_in(dir.path())
        .args([
            "job",
            "run",
            &job_id,
            "--input",
            &format!("task_id={task_id}"),
            "--input",
            "status=review",
            "--input",
            "execution_summary=Implemented change and validated tests.",
            "--input",
            "comment=Ready for review",
            "--input",
            "note=handoff ready",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["state"], "success");

    let updated = show_task(dir.path(), &task_id);
    assert_eq!(updated["status"], "review");
    assert_eq!(
        updated["execution_summary"],
        "Implemented change and validated tests."
    );
    assert!(
        updated["comments"]
            .as_array()
            .expect("comments")
            .iter()
            .any(|comment| comment["message"] == "Ready for review")
    );
    let history = updated["history"].as_array().expect("history");
    assert_eq!(
        history.last().expect("history entry")["event"],
        "status_changed"
    );
    assert_eq!(
        history.last().expect("history entry")["note"],
        "handoff ready"
    );
    assert_eq!(
        history.last().expect("history entry")["from_status"],
        "in_progress"
    );
    assert_eq!(
        history.last().expect("history entry")["to_status"],
        "review"
    );
}

#[test]
fn update_task_activity_requires_execution_summary_for_review() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_orbit(dir.path());

    let task = add_task(dir.path(), "Need summary");
    let task_id = task["id"].as_str().expect("task id").to_string();

    orbit_in(dir.path())
        .args(["task", "start", &task_id, "--json"])
        .assert()
        .success();

    let job_id = add_job(dir.path(), "update_task", "");
    let run_output = orbit_in(dir.path())
        .args([
            "job",
            "run",
            &job_id,
            "--input",
            &format!("task_id={task_id}"),
            "--input",
            "status=review",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["state"], "failed");
    assert!(
        run["error_message"]
            .as_str()
            .expect("error message")
            .contains("requires non-empty execution_summary")
    );
}

#[test]
fn update_task_activity_rejects_invalid_status_transition() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_orbit(dir.path());

    let task = add_task(dir.path(), "Wrong transition");
    let task_id = task["id"].as_str().expect("task id").to_string();

    let job_id = add_job(dir.path(), "update_task", "");
    let run_output = orbit_in(dir.path())
        .args([
            "job",
            "run",
            &job_id,
            "--input",
            &format!("task_id={task_id}"),
            "--input",
            "status=done",
            "--input",
            "execution_summary=Should not matter",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["state"], "failed");
    assert!(
        run["error_message"]
            .as_str()
            .expect("error message")
            .contains("invalid status transition")
    );
}

#[test]
fn activity_workspace_path_is_stored_and_clearable() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Add activity with workspace_path
    orbit_in(dir.path())
        .args([
            "activity",
            "add",
            "--id",
            "spec-ws-path",
            "--type",
            "cli_command",
            "--description",
            "workspace path test",
            "--spec-config",
            "{\"command\":\"cargo\",\"args\":[\"test\"]}",
            "--workspace-path",
            "/tmp/myrepo",
            "--json",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-ws-path", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["workspace_path"], "/tmp/myrepo");

    // Update to clear workspace_path
    orbit_in(dir.path())
        .args([
            "activity",
            "update",
            "spec-ws-path",
            "--clear-workspace-path",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["activity", "show", "spec-ws-path", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert!(show["workspace_path"].is_null());
}
