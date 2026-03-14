use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use orbit_types::OrbitError;

use super::contracts::LockStoreBackend;

const GLOBAL_JOB_LOCK: &str = "job/run";

#[derive(Clone, Default)]
pub(crate) struct MemoryLockStoreBackend {
    locks: Arc<Mutex<HashSet<String>>>,
}

impl LockStoreBackend for MemoryLockStoreBackend {
    fn try_lock(&self, name: &str) -> Result<bool, OrbitError> {
        let mut locks = self
            .locks
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        Ok(locks.insert(name.to_string()))
    }

    fn unlock(&self, name: &str) -> Result<bool, OrbitError> {
        let mut locks = self
            .locks
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        Ok(locks.remove(name))
    }

    fn global_job_lock_name(&self) -> &'static str {
        GLOBAL_JOB_LOCK
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryLockStoreBackend;
    use crate::backend::contracts::LockStoreBackend;

    #[test]
    fn lock_is_advisory_and_exclusive() {
        let store = MemoryLockStoreBackend::default();

        assert!(store.try_lock("abc").expect("first lock"));
        assert!(!store.try_lock("abc").expect("second lock fails"));
        assert!(store.unlock("abc").expect("unlock"));
        assert!(store.try_lock("abc").expect("lock again"));
    }
}
