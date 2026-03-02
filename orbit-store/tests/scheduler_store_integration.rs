use chrono::Utc;
use orbit_store::Store;
use orbit_types::{
    SchedulerRetryBackoffStrategy, SchedulerRunState, SchedulerScheduleState, SchedulerTargetType,
};

#[test]
fn scheduler_state_transitions_and_disabled_visibility() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let scheduler = store
        .with_transaction(|tx| {
            tx.insert_job_v2(
                SchedulerTargetType::Job,
                "spec-demo",
                "every 1h",
                "mock-agent",
                300,
                0,
                SchedulerRetryBackoffStrategy::None,
                0,
                now,
            )
        })
        .expect("insert scheduler");

    let due = store.due_schedulers(now).expect("due schedulers");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].scheduler_id, scheduler.scheduler_id);

    store
        .with_transaction(|tx| {
            tx.set_scheduler_state(&scheduler.scheduler_id, SchedulerScheduleState::Paused)
        })
        .expect("pause scheduler");
    let paused = store
        .get_scheduler(&scheduler.scheduler_id)
        .expect("get paused")
        .expect("scheduler");
    assert_eq!(paused.state, SchedulerScheduleState::Paused);

    store
        .with_transaction(|tx| {
            tx.set_scheduler_state(&scheduler.scheduler_id, SchedulerScheduleState::Enabled)
        })
        .expect("resume scheduler");
    store
        .with_transaction(|tx| tx.mark_scheduler_disabled(&scheduler.scheduler_id))
        .expect("disable scheduler");

    let default_list = store.list_schedulers(false).expect("list enabled/paused");
    assert!(
        default_list
            .iter()
            .all(|item| item.scheduler_id != scheduler.scheduler_id)
    );

    let all_list = store.list_schedulers(true).expect("list all");
    let disabled = all_list
        .iter()
        .find(|item| item.scheduler_id == scheduler.scheduler_id)
        .expect("disabled present");
    assert_eq!(disabled.state, SchedulerScheduleState::Disabled);
}

#[test]
fn claim_due_schedulers_skips_when_pending_or_running_run_exists() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let scheduler = store
        .with_transaction(|tx| {
            tx.insert_job_v2(
                SchedulerTargetType::Job,
                "spec-claim",
                "every 1m",
                "mock-agent",
                300,
                0,
                SchedulerRetryBackoffStrategy::None,
                0,
                now,
            )
        })
        .expect("insert scheduler");

    let first = store
        .with_transaction(|tx| tx.claim_due_schedulers(now))
        .expect("first claim");
    assert_eq!(first.claimed.len(), 1);
    assert!(first.skipped.is_empty());
    assert_eq!(
        first.claimed[0].scheduler.scheduler_id,
        scheduler.scheduler_id
    );
    assert_eq!(first.claimed[0].run.state, SchedulerRunState::Pending);

    let second = store
        .with_transaction(|tx| tx.claim_due_schedulers(now))
        .expect("second claim");
    assert!(second.claimed.is_empty());
    assert_eq!(second.skipped, vec![scheduler.scheduler_id.clone()]);
}

#[test]
fn complete_scheduler_run_updates_terminal_state_and_error_fields() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let scheduler = store
        .with_transaction(|tx| {
            tx.insert_job_v2(
                SchedulerTargetType::Job,
                "spec-legacy",
                "every 1m",
                "mock-agent",
                300,
                0,
                SchedulerRetryBackoffStrategy::None,
                0,
                now,
            )
        })
        .expect("insert scheduler");

    let run = store
        .with_transaction(|tx| tx.insert_scheduler_run(&scheduler.scheduler_id, 1, now))
        .expect("insert run");
    assert_eq!(run.state, SchedulerRunState::Pending);

    let started_at = Utc::now();
    store
        .with_transaction(|tx| tx.mark_scheduler_run_running(&run.run_id, started_at))
        .expect("mark running");

    store
        .with_transaction(|tx| {
            tx.complete_scheduler_run(
                &run.run_id,
                SchedulerRunState::Failed,
                Utc::now(),
                Some(1200),
                Some(130),
                None,
                Some("RUN_FAILED"),
                Some("cancel requested"),
            )
        })
        .expect("finish session");

    let finished = store
        .get_job_run(&run.run_id)
        .expect("get run")
        .expect("run");
    assert_eq!(finished.state, SchedulerRunState::Failed);
    assert_eq!(finished.duration_ms, Some(1200));
    assert_eq!(finished.exit_code, Some(130));
    assert_eq!(finished.error_code.as_deref(), Some("RUN_FAILED"));
    assert_eq!(finished.error_message.as_deref(), Some("cancel requested"));
}
