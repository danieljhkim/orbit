use orbit_core::OrbitRuntime;
use orbit_core::command::task::TaskAddParams;
use tempfile::tempdir;

#[test]
fn legacy_skill_mutation_commands_are_disabled() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let add_err = runtime
        .add_skill(orbit_core::command::skill::SkillAddParams {
            name: "legacy".to_string(),
            description: Some("desc".to_string()),
            instructions: "instructions".to_string(),
            context_files: vec![],
            allowed_tools: vec![],
            role: orbit_types::Role::Agent,
        })
        .expect_err("add should be disabled");
    assert!(
        add_err
            .to_string()
            .contains("legacy skill mutation is disabled")
    );

    let update_err = runtime
        .update_skill(
            "legacy",
            orbit_core::command::skill::SkillUpdateParams::default(),
        )
        .expect_err("update should be disabled");
    assert!(
        update_err
            .to_string()
            .contains("legacy skill mutation is disabled")
    );

    let delete_err = runtime
        .delete_skill("legacy")
        .expect_err("delete should be disabled");
    assert!(
        delete_err
            .to_string()
            .contains("legacy skill mutation is disabled")
    );
}

#[test]
fn legacy_task_skill_attachment_commands_are_disabled() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "task".to_string(),
            ..Default::default()
        })
        .expect("task");

    let attach_err = runtime
        .attach_skill_to_task(&task.id, "legacy")
        .expect_err("attach should be disabled");
    assert!(
        attach_err
            .to_string()
            .contains("task-attached skill runtime is disabled")
    );

    let list_err = runtime
        .list_task_skills(&task.id)
        .expect_err("list should be disabled");
    assert!(
        list_err
            .to_string()
            .contains("task-attached skill runtime is disabled")
    );

    let detach_err = runtime
        .detach_skill_from_task(&task.id, "legacy")
        .expect_err("detach should be disabled");
    assert!(
        detach_err
            .to_string()
            .contains("task-attached skill runtime is disabled")
    );
}

#[test]
fn file_skill_catalog_commands_work() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let skill_dir = dir.path().join("skills").join("assess-codebase");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"# assess-codebase

## Purpose
Review codebase architecture.

## Behavioral Constraints
- Return deterministic output.

## Output Requirements
- JSON response.
"#,
    )
    .expect("write skill");
    std::fs::write(
        skill_dir.join("meta.json"),
        r#"{
  "name": "Assess Codebase",
  "version": "1.0.0",
  "type": "object",
  "required": ["summary"],
  "properties": {
    "summary": {"type": "string"}
  }
}"#,
    )
    .expect("write meta");

    let listed = runtime.list_file_skills().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "assess-codebase");
    assert!(listed[0].output_schema.is_some());

    let shown = runtime.show_file_skill("assess-codebase").expect("show");
    assert_eq!(shown.sections.purpose, "Review codebase architecture.");
    assert_eq!(
        shown.meta.and_then(|meta| meta.name).as_deref(),
        Some("Assess Codebase")
    );

    let doctor = runtime.doctor_file_skills().expect("doctor");
    assert_eq!(doctor.len(), 1);
    assert_eq!(
        doctor[0].status,
        orbit_core::command::skill::SkillDoctorStatus::Ok
    );
}
