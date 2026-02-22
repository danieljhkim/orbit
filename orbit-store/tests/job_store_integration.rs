use chrono::Utc;
use orbit_store::Store;
use orbit_store::task_store::TaskInsertParams;
use orbit_types::{JobScheduleState, JobSessionStatus, JobTrigger, Role};

fn create_task_id(store: &Store, title: &str) -> String {
    store
        .with_transaction(|tx| {
            tx.insert_task(&TaskInsertParams {
                title: title.to_string(),
                ..Default::default()
            })
        })
        .expect("insert task")
        .id
}

#[test]
fn job_state_transitions_and_soft_delete_visibility() {
    let store = Store::open_in_memory().expect("store");
    let task_id = create_task_id(&store, "job task");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| tx.insert_job("nightly", &task_id, "every 1h", "UTC", Some(now)))
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
    assert!(paused.paused_at.is_some());

    store
        .with_transaction(|tx| tx.set_job_state(&job.job_id, JobScheduleState::Active))
        .expect("resume job");
    store
        .with_transaction(|tx| tx.mark_job_deleted(&job.job_id))
        .expect("delete job");

    let default_list = store.list_jobs(false).expect("list active");
    assert!(default_list.iter().all(|item| item.job_id != job.job_id));

    let all_list = store.list_jobs(true).expect("list all");
    let deleted = all_list
        .iter()
        .find(|item| item.job_id == job.job_id)
        .expect("deleted present");
    assert_eq!(deleted.state, JobScheduleState::Deleted);
    assert!(deleted.deleted_at.is_some());
}

#[test]
fn claim_due_jobs_skips_when_running_session_exists() {
    let store = Store::open_in_memory().expect("store");
    let task_id = create_task_id(&store, "job task");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| tx.insert_job("claim-test", &task_id, "every 1m", "UTC", Some(now)))
        .expect("insert job");

    let first = store
        .with_transaction(|tx| tx.claim_due_jobs(now))
        .expect("first claim");
    assert_eq!(first.claimed.len(), 1);
    assert!(first.skipped.is_empty());
    assert_eq!(first.claimed[0].job.job_id, job.job_id);
    assert_eq!(first.claimed[0].session.trigger, JobTrigger::Schedule);

    let second = store
        .with_transaction(|tx| tx.claim_due_jobs(now))
        .expect("second claim");
    assert!(second.claimed.is_empty());
    assert_eq!(second.skipped, vec![job.job_id.clone()]);
}

#[test]
fn cancel_request_sets_flag_on_running_session() {
    let store = Store::open_in_memory().expect("store");
    let task_id = create_task_id(&store, "job task");
    let now = Utc::now();

    let job = store
        .with_transaction(|tx| tx.insert_job("cancel-test", &task_id, "every 1m", "UTC", Some(now)))
        .expect("insert job");

    let session = store
        .with_transaction(|tx| {
            tx.insert_job_session(
                &job.job_id,
                &task_id,
                JobTrigger::Manual,
                Role::Admin,
                now,
                None,
                None,
            )
        })
        .expect("insert session");

    let requested = store
        .with_transaction(|tx| tx.request_cancel_running_session(&job.job_id))
        .expect("request cancel")
        .expect("running session");
    assert_eq!(requested, session.session_id);
    assert!(
        store
            .is_job_session_cancel_requested(&session.session_id)
            .expect("cancel requested")
    );

    store
        .with_transaction(|tx| {
            tx.finish_job_session(
                &session.session_id,
                JobSessionStatus::Cancelled,
                Some(130),
                Some("cancel requested"),
            )
        })
        .expect("finish cancelled");

    let finished = store
        .get_job_session(&session.session_id)
        .expect("get session")
        .expect("session");
    assert_eq!(finished.status, JobSessionStatus::Cancelled);
    assert_eq!(finished.exit_code, Some(130));
}
