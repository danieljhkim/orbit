#![allow(missing_docs)]

use super::*;

use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestCaseError};
use std::collections::HashMap;
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

const WORKERS: usize = 4;
const ATTEMPTS_PER_WORKER: usize = 16;
const TOTAL_ATTEMPTS: usize = WORKERS * ATTEMPTS_PER_WORKER;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct WorkerStats {
    acquired: usize,
    contended: usize,
}

fn release_schedule() -> impl Strategy<Value = Vec<bool>> {
    prop::collection::vec(any::<bool>(), TOTAL_ATTEMPTS..TOTAL_ATTEMPTS + 1)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4, .. ProptestConfig::default() })]

    #[test]
    fn concurrent_graph_locks_remain_loadable_and_single_owner(
        release_schedule in release_schedule()
    ) {
        run_concurrent_lock_scenario(release_schedule)?;
    }
}

fn run_concurrent_lock_scenario(release_schedule: Vec<bool>) -> Result<(), TestCaseError> {
    let temp = tempdir().map_err(|error| TestCaseError::fail(error.to_string()))?;
    let knowledge_dir = Arc::new(temp.path().to_path_buf());
    let selector = Arc::new("file:src/shared.rs".to_string());
    let active_owners = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let barrier = Arc::new(Barrier::new(WORKERS + 1));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();

    for worker in 0..WORKERS {
        let knowledge_dir = Arc::clone(&knowledge_dir);
        let selector = Arc::clone(&selector);
        let active_owners = Arc::clone(&active_owners);
        let barrier = Arc::clone(&barrier);
        let tx = tx.clone();
        let worker_schedule = release_schedule
            .iter()
            .skip(worker * ATTEMPTS_PER_WORKER)
            .take(ATTEMPTS_PER_WORKER)
            .copied()
            .collect::<Vec<_>>();

        handles.push(thread::spawn(move || {
            barrier.wait();
            let outcome = worker_lock_loop(
                worker,
                &knowledge_dir,
                &selector,
                &active_owners,
                &worker_schedule,
            );
            let _ = tx.send(outcome);
        }));
    }
    drop(tx);

    barrier.wait();
    let deadline = Instant::now() + DEADLINE;
    let mut total = WorkerStats::default();

    for _ in 0..WORKERS {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        let worker_result = rx.recv_timeout(remaining).map_err(|error| {
            TestCaseError::fail(format!(
                "lock workers did not finish within 5 seconds: {error}"
            ))
        })?;
        let stats = worker_result.map_err(TestCaseError::fail)?;
        total.acquired += stats.acquired;
        total.contended += stats.contended;
    }

    for handle in handles {
        handle
            .join()
            .map_err(|_| TestCaseError::fail("lock worker panicked"))?;
    }

    let active = active_owners
        .lock()
        .map_err(|error| TestCaseError::fail(format!("active owner registry poisoned: {error}")))?;
    prop_assert!(
        active.is_empty(),
        "active owners remained after workers: {active:?}"
    );
    drop(active);

    let store = LockStore::load(&lock_store_path(&knowledge_dir))
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    prop_assert!(
        store.locks.is_empty(),
        "all worker locks should have been released: {:?}",
        store.locks
    );
    prop_assert!(
        total.acquired > 0,
        "workers should acquire at least one lock"
    );
    prop_assert!(
        total.contended > 0,
        "shared selector should produce lock contention"
    );
    Ok(())
}

fn worker_lock_loop(
    worker: usize,
    knowledge_dir: &Path,
    selector: &str,
    active_owners: &Mutex<HashMap<String, String>>,
    release_schedule: &[bool],
) -> Result<WorkerStats, String> {
    let owner = format!("worker-{worker}");
    let mut stats = WorkerStats::default();

    for release_explicitly in release_schedule {
        let requested_selectors = [selector.to_string()];
        match GraphLockGuard::acquire(
            knowledge_dir,
            &owner,
            Some("ORB-00002"),
            "concurrency property test",
            &requested_selectors,
        ) {
            Ok(mut guard) => {
                stats.acquired += 1;
                mark_active(active_owners, selector, &owner)?;
                assert_store_owner(knowledge_dir, selector, &owner)?;
                thread::yield_now();
                assert_store_owner(knowledge_dir, selector, &owner)?;
                mark_inactive(active_owners, selector, &owner)?;
                if *release_explicitly {
                    guard.release().map_err(|error| error.to_string())?;
                }
            }
            Err(error) if error.kind == crate::error::KnowledgeErrorKind::Invalid => {
                stats.contended += 1;
                thread::yield_now();
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    Ok(stats)
}

fn mark_active(
    active_owners: &Mutex<HashMap<String, String>>,
    selector: &str,
    owner: &str,
) -> Result<(), String> {
    let mut active = active_owners
        .lock()
        .map_err(|error| format!("active owner registry poisoned: {error}"))?;
    if let Some(previous_owner) = active.insert(selector.to_string(), owner.to_string()) {
        return Err(format!(
            "selector `{selector}` was active for both `{previous_owner}` and `{owner}`"
        ));
    }
    Ok(())
}

fn mark_inactive(
    active_owners: &Mutex<HashMap<String, String>>,
    selector: &str,
    owner: &str,
) -> Result<(), String> {
    let mut active = active_owners
        .lock()
        .map_err(|error| format!("active owner registry poisoned: {error}"))?;
    match active.remove(selector) {
        Some(active_owner) if active_owner == owner => Ok(()),
        Some(active_owner) => Err(format!(
            "selector `{selector}` was active for `{active_owner}` while `{owner}` held the guard"
        )),
        None => Err(format!(
            "selector `{selector}` was missing from active owners while `{owner}` held the guard"
        )),
    }
}

fn assert_store_owner(knowledge_dir: &Path, selector: &str, owner: &str) -> Result<(), String> {
    let store =
        LockStore::load(&lock_store_path(knowledge_dir)).map_err(|error| error.to_string())?;
    match store.get(selector) {
        Some(lock) if lock.owner == owner => Ok(()),
        Some(lock) => Err(format!(
            "selector `{selector}` was stored for `{}` while `{owner}` held the guard",
            lock.owner
        )),
        None => Err(format!(
            "selector `{selector}` was missing from the lock store while `{owner}` held the guard"
        )),
    }
}
