use chrono::Utc;
use orbit_store::Store;
use orbit_store::task_store::TaskInsertParams;

#[test]
fn due_jobs_query_returns_scheduled_jobs() {
    let store = Store::open_in_memory().expect("store");
    let next_run = Utc::now();

    store
        .with_transaction(|tx| {
            let task = tx.insert_task(&TaskInsertParams {
                title: "job task".to_string(),
                ..Default::default()
            })?;
            let _job = tx.insert_job("nightly", &task.id, "every 1h", "UTC", Some(next_run))?;
            Ok(())
        })
        .expect("insert job");

    let due = store.due_jobs(next_run).expect("due jobs");
    assert_eq!(due.len(), 1);
}
