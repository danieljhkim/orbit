use crate::driver::sqlite::routine_store::try_acquire_routine_sweep_lock;
use crate::{RoutineFireIntentParams, RoutineFireState, Store};

fn store() -> Store {
    Store::open_in_memory().expect("in-memory store")
}

fn intent(name: &str, slot: &str, attempt: u32) -> RoutineFireIntentParams {
    RoutineFireIntentParams {
        routine_name: name.to_string(),
        slot: slot.to_string(),
        attempt,
        source_workspace: "polaris".to_string(),
    }
}

#[test]
fn baseline_is_idempotent_and_never_moves() {
    let store = store();
    assert!(
        store
            .routine_record_baseline("nightly", "2026-07-01T00:00:00+00:00")
            .expect("first baseline")
    );
    assert!(
        !store
            .routine_record_baseline("nightly", "2026-07-02T00:00:00+00:00")
            .expect("second baseline")
    );
    let cursor = store
        .routine_cursor("nightly")
        .expect("cursor")
        .expect("cursor exists");
    assert_eq!(cursor.baseline_at, "2026-07-01T00:00:00+00:00");
    assert_eq!(cursor.last_slot, None);
}

#[test]
fn fire_intent_is_idempotent_per_slot_and_advances_cursor() {
    let store = store();
    store
        .routine_record_baseline("nightly", "2026-07-01T00:00:00+00:00")
        .expect("baseline");

    let slot = "2026-07-02T22:00:00+00:00";
    assert!(
        store
            .routine_record_fire_intent(&intent("nightly", slot, 1))
            .expect("first claim")
    );
    // A second sweep racing on the same slot must not claim it again.
    assert!(
        !store
            .routine_record_fire_intent(&intent("nightly", slot, 1))
            .expect("second claim")
    );

    let cursor = store
        .routine_cursor("nightly")
        .expect("cursor")
        .expect("cursor exists");
    assert_eq!(cursor.last_slot.as_deref(), Some(slot));

    // A retry of the same slot is a distinct attempt, not a duplicate.
    assert!(
        store
            .routine_record_fire_intent(&intent("nightly", slot, 2))
            .expect("retry claim")
    );
}

#[test]
fn fire_lifecycle_updates_state_run_id_and_detail() {
    let store = store();
    let slot = "2026-07-02T22:00:00+00:00";
    store
        .routine_record_fire_intent(&intent("nightly", slot, 1))
        .expect("claim");

    store
        .routine_mark_fire_dispatched("nightly", slot, 1, "run-123")
        .expect("dispatched");
    let fire = store
        .routine_latest_fire("nightly")
        .expect("latest")
        .expect("fire exists");
    assert_eq!(fire.state, RoutineFireState::Dispatched);
    assert_eq!(fire.run_id.as_deref(), Some("run-123"));
    assert!(!fire.state.is_terminal());

    store
        .routine_mark_fire_outcome("nightly", slot, 1, RoutineFireState::Failed, Some("exit 1"))
        .expect("outcome");
    let fire = store
        .routine_latest_fire("nightly")
        .expect("latest")
        .expect("fire exists");
    assert_eq!(fire.state, RoutineFireState::Failed);
    // run_id survives the outcome update (COALESCE keeps the old value).
    assert_eq!(fire.run_id.as_deref(), Some("run-123"));
    assert_eq!(fire.detail.as_deref(), Some("exit 1"));
    assert!(fire.state.is_terminal());
}

#[test]
fn unresolved_fires_lists_only_non_terminal_states() {
    let store = store();
    store
        .routine_record_fire_intent(&intent("a", "2026-07-02T01:00:00+00:00", 1))
        .expect("claim a");
    store
        .routine_record_fire_intent(&intent("b", "2026-07-02T02:00:00+00:00", 1))
        .expect("claim b");
    store
        .routine_mark_fire_dispatched("b", "2026-07-02T02:00:00+00:00", 1, "run-b")
        .expect("dispatch b");
    store
        .routine_record_fire_intent(&intent("c", "2026-07-02T03:00:00+00:00", 1))
        .expect("claim c");
    store
        .routine_mark_fire_outcome(
            "c",
            "2026-07-02T03:00:00+00:00",
            1,
            RoutineFireState::Succeeded,
            None,
        )
        .expect("resolve c");

    let unresolved = store.routine_unresolved_fires().expect("unresolved");
    let names: Vec<&str> = unresolved
        .iter()
        .map(|fire| fire.routine_name.as_str())
        .collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn latest_fire_orders_by_slot_then_attempt() {
    let store = store();
    for (slot, attempt) in [
        ("2026-07-01T22:00:00+00:00", 1),
        ("2026-07-02T22:00:00+00:00", 1),
        ("2026-07-02T22:00:00+00:00", 2),
    ] {
        store
            .routine_record_fire_intent(&intent("nightly", slot, attempt))
            .expect("claim");
    }
    let latest = store
        .routine_latest_fire("nightly")
        .expect("latest")
        .expect("fire exists");
    assert_eq!(latest.slot, "2026-07-02T22:00:00+00:00");
    assert_eq!(latest.attempt, 2);

    let recent = store.routine_recent_fires("nightly", 2).expect("recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].attempt, 2);
    assert_eq!(recent[1].attempt, 1);
}

#[test]
fn pause_and_resume_round_trip() {
    let store = store();
    assert!(store.routine_pause("nightly", "human").expect("pause"));
    assert!(!store.routine_pause("nightly", "human").expect("re-pause"));

    let pauses = store.routine_pauses().expect("pauses");
    assert!(pauses.contains_key("nightly"));
    assert_eq!(pauses["nightly"].actor.as_deref(), Some("human"));

    assert!(store.routine_resume("nightly").expect("resume"));
    assert!(!store.routine_resume("nightly").expect("re-resume"));
    assert!(store.routine_pauses().expect("pauses").is_empty());
}

#[test]
fn sweep_lock_is_exclusive_per_path_and_released_on_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = try_acquire_routine_sweep_lock(dir.path())
        .expect("acquire")
        .expect("lock free");
    assert!(
        try_acquire_routine_sweep_lock(dir.path())
            .expect("second acquire")
            .is_none(),
        "held lock must not be re-acquirable"
    );
    drop(first);
    assert!(
        try_acquire_routine_sweep_lock(dir.path())
            .expect("third acquire")
            .is_some(),
        "dropped lock must be re-acquirable"
    );
}
