//! Manual-mint tests [ORB-10439]: `auto_task_generate` is unconditional
//! (ignores schedule due-math, `dedupe`, and `enabled`), leaves the host-local
//! cursor untouched, and produces a task field-for-field identical to a
//! scheduler fire.

use chrono::{DateTime, Duration, TimeZone, Utc};
use orbit_common::types::{DedupePolicy, Task, TaskStatus, auto_task_tag};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::auto_tasks::scheduler::{SchedulerOptions, run_auto_task_scheduler_at};
use crate::auto_tasks::state::cursor_state_path;

use super::interval_params;

fn runtime() -> OrbitRuntime {
    OrbitRuntime::in_memory().expect("build in-memory runtime")
}

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .expect("valid ts")
}

/// One scheduler pass, projected to `(action, task_id)` per definition.
fn fire(runtime: &OrbitRuntime, now: DateTime<Utc>) -> Vec<(String, Option<String>)> {
    let outcome = run_auto_task_scheduler_at(runtime, now, SchedulerOptions::default())
        .expect("scheduler pass");
    outcome
        .reports
        .iter()
        .map(|report| (report.action.to_string(), report.task_id.clone()))
        .collect()
}

/// Raw bytes of the cursor state file, or `None` when it does not exist.
fn cursor_bytes(runtime: &OrbitRuntime) -> Option<Vec<u8>> {
    std::fs::read(cursor_state_path(&runtime.paths().state_dir)).ok()
}

/// A task's JSON projection with the identity/clock fields stripped, so two
/// tasks minted at different moments compare on their template-derived content
/// alone — and any future `Task` field the mint path starts populating is
/// covered without touching this helper.
fn content(task: &Task) -> Value {
    let mut value = serde_json::to_value(task).expect("task json");
    let object = value.as_object_mut().expect("task json object");
    for volatile in ["id", "created_at", "updated_at"] {
        object.remove(volatile);
    }
    value
}

#[test]
fn generate_matches_a_scheduler_fire_field_for_field() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");
    let t0 = at(2026, 1, 1, 0, 0);

    fire(&runtime, t0); // baseline
    let reports = fire(&runtime, t0 + Duration::minutes(65));
    assert_eq!(reports[0].0, "fired");
    let fired = runtime
        .get_task(&reports[0].1.clone().expect("task id"))
        .expect("fired task");

    let generated = runtime.auto_task_generate("chore").expect("generate");

    assert_ne!(generated.id, fired.id, "generate mints a distinct task");
    assert_eq!(
        content(&generated),
        content(&fired),
        "a generated task must be indistinguishable from a fired one"
    );
    assert!(
        generated.tags.contains(&auto_task_tag("chore")),
        "expected provenance tag, got {:?}",
        generated.tags
    );
    assert_eq!(
        generated.created_by.as_deref(),
        Some("system"),
        "generate mints with the system_created identity"
    );
    assert_eq!(
        generated.status,
        TaskStatus::Backlog,
        "generate honors the template-supplied status default"
    );
}

#[test]
fn generate_succeeds_for_a_disabled_definition() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");
    runtime
        .auto_task_toggle("chore", false)
        .expect("toggle off");

    let generated = runtime.auto_task_generate("chore").expect("generate");
    assert!(generated.tags.contains(&auto_task_tag("chore")));
    assert_eq!(runtime.list_tasks().expect("tasks").len(), 1);
}

#[test]
fn generate_succeeds_while_an_instance_is_open_under_skip_if_open() {
    let runtime = runtime();
    let params = interval_params("chore", 60);
    assert_eq!(params.dedupe, DedupePolicy::SkipIfOpen);
    runtime.auto_task_add(params).expect("add");

    let first = runtime.auto_task_generate("chore").expect("generate");
    assert!(first.tags.contains(&auto_task_tag("chore")));

    // The first instance is open (backlog), yet a second manual mint still lands.
    let second = runtime.auto_task_generate("chore").expect("generate again");
    assert_ne!(first.id, second.id);
    assert_eq!(runtime.list_tasks().expect("tasks").len(), 2);
}

#[test]
fn generate_leaves_the_cursor_byte_identical() {
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");
    let t0 = at(2026, 1, 1, 0, 0);

    // Before any scheduler pass the file does not exist — generate must not
    // create it.
    assert!(cursor_bytes(&runtime).is_none());
    runtime.auto_task_generate("chore").expect("generate");
    assert!(
        cursor_bytes(&runtime).is_none(),
        "generate must not create cursor state"
    );

    fire(&runtime, t0); // baseline writes the cursor
    let before = cursor_bytes(&runtime).expect("cursor state after baseline");
    runtime.auto_task_generate("chore").expect("generate");
    assert_eq!(
        cursor_bytes(&runtime).expect("cursor state after generate"),
        before,
        "generate must not read-modify-write the cursor"
    );
}

#[test]
fn generate_does_not_change_the_next_scheduler_decision() {
    // `always` dedupe isolates the cursor question from the provenance-tag
    // question: the only thing that could move the next pass is the cursor.
    let mut params = interval_params("chore", 60);
    params.dedupe = DedupePolicy::Always;
    let t0 = at(2026, 1, 1, 0, 0);

    let control = runtime();
    control.auto_task_add(params.clone()).expect("add");
    fire(&control, t0); // baseline
    let control_outcome = run_auto_task_scheduler_at(
        &control,
        t0 + Duration::minutes(65),
        SchedulerOptions::default(),
    )
    .expect("control pass");

    let generated = runtime();
    generated.auto_task_add(params).expect("add");
    fire(&generated, t0); // baseline
    generated.auto_task_generate("chore").expect("generate");
    let generated_outcome = run_auto_task_scheduler_at(
        &generated,
        t0 + Duration::minutes(65),
        SchedulerOptions::default(),
    )
    .expect("pass after generate");

    assert_eq!(control_outcome.reports[0].action, "fired");
    assert_eq!(
        generated_outcome.reports[0].action,
        control_outcome.reports[0].action,
    );
    assert_eq!(
        generated_outcome.reports[0].slot, control_outcome.reports[0].slot,
        "the consumed slot must be unaffected by a manual mint"
    );
}

#[test]
fn an_open_generated_instance_defers_the_next_fire_like_a_fired_one() {
    // The flip side of provenance parity: a generated task carries the
    // `auto-task:<name>` tag, so `skip_if_open` sees it exactly as it sees a
    // fired instance. That is the point — a hand-copied task is invisible to
    // dedupe, and this surface exists so a manual mint is not.
    let runtime = runtime();
    runtime
        .auto_task_add(interval_params("chore", 60))
        .expect("add");
    let t0 = at(2026, 1, 1, 0, 0);

    fire(&runtime, t0); // baseline
    let generated = runtime.auto_task_generate("chore").expect("generate");

    let deferred = fire(&runtime, t0 + Duration::minutes(65));
    assert_eq!(deferred[0].0, "skipped");
    assert_eq!(runtime.list_tasks().expect("tasks").len(), 1);

    // The cursor never advanced, so the pending occurrence fires (once) the
    // moment the queue drains — the same recovery as a stalled fired instance.
    runtime
        .update_task(
            &generated.id,
            crate::command::task::TaskUpdateParams {
                status: Some(TaskStatus::Rejected),
                ..Default::default()
            },
        )
        .expect("close generated task");
    let drained = fire(&runtime, t0 + Duration::minutes(125));
    assert_eq!(drained[0].0, "fired");
    assert_eq!(runtime.list_tasks().expect("tasks").len(), 2);
}

#[test]
fn generate_with_an_unknown_name_errors_naming_the_definition() {
    let runtime = runtime();
    let error = runtime
        .auto_task_generate("nope")
        .expect_err("unknown definition must fail");
    assert!(
        error.to_string().contains("nope"),
        "error must name the definition, got: {error}"
    );
    assert!(runtime.list_tasks().expect("tasks").is_empty());
}
