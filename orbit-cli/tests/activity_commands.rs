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
            "analysis",
            "--description",
            "CLI activity test",
            "--instruction",
            "Inspect repository metrics and summarize them.",
            "--input-schema",
            "{\"type\":\"object\"}",
            "--output-schema",
            "{\"type\":\"object\"}",
            "--skill-refs",
            "orbit-assess-codebase,execution-audit",
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
    assert_eq!(show["type"], "analysis");
    assert_eq!(
        show["instruction"],
        "Inspect repository metrics and summarize them."
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
    assert_eq!(show["type"], "general");
    assert_eq!(show["instruction"], "");
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

    orbit_in(dir.path())
        .args(["init"])
        .assert()
        .success();

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
    assert!(activity.get("instruction").is_none());
    assert!(activity.get("input_schema_json").is_none());
    assert!(activity.get("output_schema_json").is_none());
    assert!(activity.get("skill_refs").is_none());
}
