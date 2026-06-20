// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use super::*;

pub(super) struct Semaphore {
    state: Mutex<usize>,
    cond: std::sync::Condvar,
}

impl Semaphore {
    pub(super) fn new(n: usize) -> Self {
        Self {
            state: Mutex::new(n),
            cond: std::sync::Condvar::new(),
        }
    }

    pub(super) fn acquire(self: &Arc<Self>) -> Permit {
        let mut guard = self.state.lock().expect("sem poisoned");
        while *guard == 0 {
            guard = self.cond.wait(guard).expect("sem poisoned");
        }
        *guard -= 1;
        Permit {
            sem: Arc::clone(self),
        }
    }

    fn release(&self) {
        let mut guard = self.state.lock().expect("sem poisoned");
        *guard += 1;
        self.cond.notify_one();
    }
}

pub(super) struct Permit {
    sem: Arc<Semaphore>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.sem.release();
    }
}
