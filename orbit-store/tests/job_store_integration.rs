use chrono::Utc;
use orbit_store::{JobRunQuery, Store};
use orbit_types::{JobRetryBackoffStrategy, JobRunState, JobScheduleState, JobTargetType};

fn insert_job<'a>(
    tx: &mut orbit_store::StoreTx<'a>,
    target_id: &str,
    next_run_at: chrono::DateTime<chrono::Utc>,
) -> Result<orbit_types::Job, orbit_types::OrbitError> {
    tx.insert_activity_v2(
        None,
        JobTargetType::Activity,
        target_id,
        "every 1m",
        "mock-agent",
        300,
        0,
        JobRetryBackoffStrategy::None,
        0,
        next_run_at,
        JobScheduleState::Enabled,
        vec![],
    )
}

#[test]
fn job_state_transitions_and_disabled_visibility() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| {
            tx.insert_activity_v2(
                None,
                JobTargetType::Activity,
                "spec-demo",
                "every 1h",
                "mock-agent",
                300,
                0,
                JobRetryBackoffStrategy::None,
                0,
                now,
                JobScheduleState::Enabled,
                vec![],
            )
        })
        .expect("insert job");

    let due = store.due_jobs(now).expect("due jobs");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].job_id, job.job_id);

    store
        .with_transaction(|tx| tx.set_job_state(&job.job_id, JobScheduleState::Paused))
        .expect("pause job");
    let paused = store
        .get_job(&job.job_id)
        .expect("get paused")
        .expect("job");
    assert_eq!(paused.state, JobScheduleState::Paused);

    store
        .with_transaction(|tx| tx.set_job_state(&job.job_id, JobScheduleState::Enabled))
        .expect("resume job");
    store
        .with_transaction(|tx| tx.mark_job_disabled(&job.job_id))
        .expect("disable job");

    let default_list = store.list_jobs(false).expect("list enabled/paused");
    assert!(default_list.iter().all(|item| item.job_id != job.job_id));

    let all_list = store.list_jobs(true).expect("list all");
    let disabled = all_list
        .iter()
        .find(|item| item.job_id == job.job_id)
        .expect("disabled present");
    assert_eq!(disabled.state, JobScheduleState::Disabled);
}

#[test]
fn claim_due_jobs_skips_when_pending_or_running_run_exists() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| insert_job(tx, "spec-claim", now))
        .expect("insert job");

    let first = store
        .with_transaction(|tx| tx.claim_due_jobs(now))
        .expect("first claim");
    assert_eq!(first.claimed.len(), 1);
    assert!(first.skipped.is_empty());
    assert_eq!(first.claimed[0].job.job_id, job.job_id);
    assert_eq!(first.claimed[0].run.state, JobRunState::Pending);

    let second = store
        .with_transaction(|tx| tx.claim_due_jobs(now))
        .expect("second claim");
    assert!(second.claimed.is_empty());
    assert_eq!(second.skipped, vec![job.job_id.clone()]);
}

#[test]
fn complete_job_run_updates_terminal_state_and_error_fields() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| insert_job(tx, "spec-legacy", now))
        .expect("insert job");

    let run = store
        .with_transaction(|tx| tx.insert_job_run(&job.job_id, 1, now))
        .expect("insert run");
    assert_eq!(run.state, JobRunState::Pending);

    let started_at = Utc::now();
    store
        .with_transaction(|tx| tx.mark_job_run_running(&run.run_id, started_at))
        .expect("mark running");

    store
        .with_transaction(|tx| {
            tx.complete_job_run(
                &run.run_id,
                JobRunState::Failed,
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
        .get_activity_run(&run.run_id)
        .expect("get run")
        .expect("run");
    assert_eq!(finished.state, JobRunState::Failed);
    assert_eq!(finished.duration_ms, Some(1200));
    assert_eq!(finished.exit_code, Some(130));
    assert_eq!(finished.error_code.as_deref(), Some("RUN_FAILED"));
    assert_eq!(finished.error_message.as_deref(), Some("cancel requested"));
}

#[test]
fn job_run_query_supports_lookup_and_filtering() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let first_job = store
        .with_transaction(|tx| insert_job(tx, "spec-query-success", now))
        .expect("insert first job");
    let second_job = store
        .with_transaction(|tx| insert_job(tx, "spec-query-failed", now))
        .expect("insert second job");

    let success_run = store
        .with_transaction(|tx| tx.insert_job_run(&first_job.job_id, 1, now))
        .expect("insert success run");
    store
        .with_transaction(|tx| {
            tx.complete_job_run(
                &success_run.run_id,
                JobRunState::Success,
                Utc::now(),
                None,
                None,
                None,
                None,
                None,
            )
        })
        .expect("complete success run");

    let failed_run = store
        .with_transaction(|tx| tx.insert_job_run(&second_job.job_id, 1, now))
        .expect("insert failed run");
    store
        .with_transaction(|tx| {
            tx.complete_job_run(
                &failed_run.run_id,
                JobRunState::Failed,
                Utc::now(),
                None,
                Some(1),
                None,
                Some("FAILED"),
                Some("run failed"),
            )
        })
        .expect("complete failed run");

    let fetched = store
        .get_job_run(&failed_run.run_id)
        .expect("lookup run")
        .expect("run exists");
    assert_eq!(fetched.run_id, failed_run.run_id);
    assert_eq!(fetched.job_id, second_job.job_id);

    let filtered = store
        .list_job_runs_filtered(&JobRunQuery {
            job_id: Some(second_job.job_id.clone()),
            state: Some(JobRunState::Failed),
            created_since: Some(now - chrono::Duration::seconds(1)),
            limit: Some(10),
        })
        .expect("filtered runs");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].run_id, failed_run.run_id);

    let limited = store
        .list_job_runs_filtered(&JobRunQuery {
            limit: Some(1),
            ..Default::default()
        })
        .expect("limited runs");
    assert_eq!(limited.len(), 1);
}

#[test]
fn archive_and_delete_job_runs_update_active_visibility() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| insert_job(tx, "spec-archive-delete", now))
        .expect("insert job");

    let archived_run = store
        .with_transaction(|tx| tx.insert_job_run(&job.job_id, 1, now))
        .expect("insert archived candidate");
    let deleted_run = store
        .with_transaction(|tx| tx.insert_job_run(&job.job_id, 2, now))
        .expect("insert deleted candidate");

    store
        .with_transaction(|tx| tx.archive_job_run(&archived_run.run_id))
        .expect("archive run");

    assert!(
        store
            .get_job_run(&archived_run.run_id)
            .expect("lookup archived run")
            .is_none(),
        "archived runs should disappear from active lookup"
    );

    let active_runs = store
        .list_job_runs(&job.job_id)
        .expect("list active runs after archive");
    assert_eq!(active_runs.len(), 1);
    assert_eq!(active_runs[0].run_id, deleted_run.run_id);

    store
        .with_transaction(|tx| tx.delete_job_run(&archived_run.run_id))
        .expect("delete archived run");
    store
        .with_transaction(|tx| tx.delete_job_run(&deleted_run.run_id))
        .expect("delete active run");

    assert!(
        store
            .list_job_runs(&job.job_id)
            .expect("list after delete")
            .is_empty(),
        "all active runs deleted"
    );
}

#[test]
fn next_due_job_time_returns_earliest_enabled_job() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();
    let earliest = now + chrono::Duration::minutes(5);
    let latest = now + chrono::Duration::minutes(15);

    let paused_id = store
        .with_transaction(|tx| {
            let paused = tx.insert_activity_v2(
                None,
                JobTargetType::Activity,
                "spec-paused",
                "every 1m",
                "mock-agent",
                300,
                0,
                JobRetryBackoffStrategy::None,
                0,
                now + chrono::Duration::minutes(1),
                JobScheduleState::Enabled,
                vec![],
            )?;
            let _ = tx.set_job_state(&paused.job_id, JobScheduleState::Paused)?;
            let _enabled = tx.insert_activity_v2(
                None,
                JobTargetType::Activity,
                "spec-enabled",
                "every 1m",
                "mock-agent",
                300,
                0,
                JobRetryBackoffStrategy::None,
                0,
                earliest,
                JobScheduleState::Enabled,
                vec![],
            )?;
            let _ = tx.insert_activity_v2(
                None,
                JobTargetType::Activity,
                "spec-later",
                "every 1m",
                "mock-agent",
                300,
                0,
                JobRetryBackoffStrategy::None,
                0,
                latest,
                JobScheduleState::Enabled,
                vec![],
            )?;
            Ok(paused.job_id)
        })
        .expect("insert jobs");

    let next_due = store.next_due_job_time().expect("next due job time");

    assert_eq!(next_due, Some(earliest));

    store
        .with_transaction(|tx| tx.mark_job_disabled(&paused_id))
        .expect("disable paused job");
    assert_eq!(
        store
            .next_due_job_time()
            .expect("next due after disabling paused"),
        Some(earliest)
    );
}

#[test]
fn next_due_job_time_returns_none_without_enabled_jobs() {
    let store = Store::open_in_memory().expect("store");

    assert_eq!(
        store.next_due_job_time().expect("next due without jobs"),
        None
    );
}
