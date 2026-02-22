use chrono::Utc;
use orbit_store::Store;

#[test]
fn due_jobs_query_returns_scheduled_jobs() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();

    store
        .with_transaction(|tx| {
            let _job = tx.insert_job("nightly", "noop", now)?;
            Ok(())
        })
        .expect("insert job");

    let due = store.due_jobs(now).expect("due jobs");
    assert_eq!(due.len(), 1);
}
