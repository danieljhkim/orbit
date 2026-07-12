//! CRUD-surface tests [ORB-10149]: add/list/show/update/toggle roundtrip,
//! duplicate rejection, fail-closed parsing, and the no-turn-knobs guarantee.

use orbit_common::types::{AutoTaskSchedule, DedupePolicy, parse_auto_task_yaml};

use crate::OrbitRuntime;
use crate::auto_tasks::crud::AutoTaskUpdateParams;

use super::{interval_params, template};

fn runtime() -> OrbitRuntime {
    OrbitRuntime::in_memory().expect("build in-memory runtime")
}

#[test]
fn add_list_show_roundtrip() {
    let runtime = runtime();
    let created = runtime
        .auto_task_add(interval_params("nightly-chore", 1440))
        .expect("add");
    assert_eq!(created.name, "nightly-chore");
    assert!(created.enabled);
    assert_eq!(created.created_by.as_deref(), Some(runtime.actor_label()));

    let listed = runtime.auto_task_list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "nightly-chore");

    let shown = runtime.auto_task_show("nightly-chore").expect("show");
    assert_eq!(shown.expect("present").name, "nightly-chore");
    assert!(
        runtime
            .auto_task_show("missing")
            .expect("show missing")
            .is_none()
    );
}

#[test]
fn add_rejects_duplicate_name() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("dup", 60))
        .expect("add");
    let err = runtime
        .auto_task_add(interval_params("dup", 60))
        .expect_err("second add rejected");
    assert!(err.to_string().contains("already exists"), "{err}");
}

#[test]
fn add_rejects_invalid_name_and_schedule() {
    let runtime = runtime();
    let mut bad_name = interval_params("placeholder", 60);
    bad_name.name = "Bad Name".to_string();
    assert!(runtime.auto_task_add(bad_name).is_err());

    let mut bad_cron = interval_params("bad-cron", 60);
    bad_cron.schedule = AutoTaskSchedule::Cron {
        cron: "nonsense".to_string(),
    };
    assert!(runtime.auto_task_add(bad_cron).is_err());
}

#[test]
fn update_patches_present_fields() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");

    let updated = runtime
        .auto_task_update(
            "chore",
            AutoTaskUpdateParams {
                description: Some("new body".to_string()),
                schedule: Some(AutoTaskSchedule::Cron {
                    cron: "0 9 * * *".to_string(),
                }),
                dedupe: Some(DedupePolicy::Always),
                template: Some(template("Renamed chore")),
            },
        )
        .expect("update");
    assert_eq!(updated.description, "new body");
    assert_eq!(updated.dedupe, DedupePolicy::Always);
    assert_eq!(updated.template.title, "Renamed chore");
    assert!(matches!(updated.schedule, AutoTaskSchedule::Cron { .. }));
    assert!(updated.updated_at >= updated.created_at);
}

#[test]
fn toggle_disables_without_deleting() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");

    let disabled = runtime
        .auto_task_toggle("chore", false)
        .expect("toggle off");
    assert!(!disabled.enabled);
    assert!(runtime.auto_task_show("chore").expect("show").is_some());

    let enabled = runtime.auto_task_toggle("chore", true).expect("toggle on");
    assert!(enabled.enabled);
}

#[test]
fn update_missing_definition_errors() {
    let runtime = runtime();
    assert!(runtime.auto_task_toggle("ghost", false).is_err());
    assert!(
        runtime
            .auto_task_update("ghost", AutoTaskUpdateParams::default())
            .is_err()
    );
}

#[test]
fn parse_rejects_turn_based_knobs_and_unknown_fields() {
    // ADR-0217: the schema is provider-neutral; a turn budget anywhere in the
    // definition (including the template) is a hard parse error.
    let with_turns = r#"
schemaVersion: 1
name: chore
schedule:
  every_minutes: 60
template:
  title: Chore
  max_turns: 40
"#;
    assert!(parse_auto_task_yaml(with_turns).is_err());

    let top_level_turns = r#"
schemaVersion: 1
name: chore
turns: 10
schedule:
  every_minutes: 60
template:
  title: Chore
"#;
    assert!(parse_auto_task_yaml(top_level_turns).is_err());
}
