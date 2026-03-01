use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
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
fn work_add_show_list_delete_json_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_skill(dir.path(), "orbit-assess-codebase");
    write_skill(dir.path(), "execution-audit");

    orbit_in(dir.path())
        .args([
            "work",
            "add",
            "--id",
            "spec-cli-1",
            "--type",
            "analysis",
            "--description",
            "CLI work test",
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
        .args(["work", "show", "spec-cli-1", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["id"], "spec-cli-1");
    assert_eq!(show["type"], "analysis");
    assert_eq!(show["is_active"], true);

    let list_output = orbit_in(dir.path())
        .args(["work", "list", "--json"])
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
        .args(["work", "delete", "spec-cli-1"])
        .assert()
        .success();

    let list_after_delete = orbit_in(dir.path())
        .args(["work", "list", "--json"])
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
fn work_add_defaults_type_and_schemas_when_omitted() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "work",
            "add",
            "--id",
            "spec-cli-defaults",
            "--description",
            "CLI work defaults test",
            "--json",
        ])
        .assert()
        .success();

    let show_output = orbit_in(dir.path())
        .args(["work", "show", "spec-cli-defaults", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["id"], "spec-cli-defaults");
    assert_eq!(show["type"], "general");
    assert_eq!(show["input_schema_json"], serde_json::json!({}));
    assert_eq!(show["output_schema_json"], serde_json::json!({}));
}

#[test]
fn workflow_command_is_not_supported() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["workflow", "list"])
        .assert()
        .failure();
}
