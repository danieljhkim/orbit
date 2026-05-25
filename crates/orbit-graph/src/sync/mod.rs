//! Graph synchronization orchestration.

pub(crate) mod pass1;
pub(crate) mod pass2;
pub(crate) mod scanner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use crate::{GraphError, SyncMode, SyncReport};

pub(crate) fn run(
    db_path: &Path,
    worktree_root: &Path,
    mode: SyncMode,
) -> Result<SyncReport, GraphError> {
    coalesced(db_path, || run_once(db_path, worktree_root, mode))
}

fn run_once(
    db_path: &Path,
    worktree_root: &Path,
    mode: SyncMode,
) -> Result<SyncReport, GraphError> {
    let started = Instant::now();
    let _lock = scanner::DbLockGuard::acquire(db_path)?;
    let diff = scanner::scan_diff_with_lock_held(db_path, worktree_root, mode)?;
    maybe_fail_after_scan(db_path)?;
    maybe_wait_after_scan(db_path);
    let pass1 = pass1::run(db_path, worktree_root, mode, &diff)?;
    pass2::run(db_path, mode, pass1.refs)?;
    let duration = started.elapsed();

    Ok(SyncReport {
        files_indexed: pass1.files_indexed,
        files_changed: pass1.files_written,
        files_removed: pass1.files_removed,
        duration,
    })
}

type SyncResult = Result<SyncReport, GraphError>;

struct InFlightSync {
    result: Mutex<Option<SyncResult>>,
    ready: Condvar,
}

fn coalesced<F>(db_path: &Path, run: F) -> SyncResult
where
    F: FnOnce() -> SyncResult,
{
    let key = db_path.to_path_buf();
    let (state, leader) = {
        let mut in_flight = in_flight_syncs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(state) = in_flight.get(&key) {
            (Arc::clone(state), false)
        } else {
            let state = Arc::new(InFlightSync {
                result: Mutex::new(None),
                ready: Condvar::new(),
            });
            in_flight.insert(key.clone(), Arc::clone(&state));
            (state, true)
        }
    };

    if leader {
        note_sync_leader_started(key.as_path());
        let result = run();
        {
            let mut slot = state
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *slot = Some(result.clone());
            state.ready.notify_all();
        }
        let mut in_flight = in_flight_syncs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        in_flight.remove(&key);
        result
    } else {
        let mut slot = state
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(result) = slot.as_ref() {
                return result.clone();
            }
            slot = state
                .ready
                .wait(slot)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

fn in_flight_syncs() -> &'static Mutex<HashMap<PathBuf, Arc<InFlightSync>>> {
    static IN_FLIGHT_SYNCS: OnceLock<Mutex<HashMap<PathBuf, Arc<InFlightSync>>>> = OnceLock::new();
    IN_FLIGHT_SYNCS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn maybe_fail_after_scan(db_path: &Path) -> Result<(), GraphError> {
    let mut paths = fail_after_scan_paths()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if paths.remove(db_path) {
        return Err(GraphError::invalid_data(
            "run graph sync",
            "injected sync failure after scan",
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_fail_after_scan(_db_path: &Path) -> Result<(), GraphError> {
    Ok(())
}

#[cfg(test)]
fn maybe_wait_after_scan(db_path: &Path) {
    if let Some(gate) = sync_after_scan_gate(db_path) {
        gate.mark_started();
        gate.wait_released();
    }
}

#[cfg(not(test))]
fn maybe_wait_after_scan(_db_path: &Path) {}

#[cfg(test)]
pub(crate) fn fail_next_sync_after_scan(db_path: &Path) {
    // L-0051: scope injected sync failures by DB path because orbit-graph tests run in parallel.
    fail_after_scan_paths()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(db_path.to_path_buf());
}

#[cfg(test)]
fn fail_after_scan_paths() -> &'static Mutex<std::collections::BTreeSet<PathBuf>> {
    static FAIL_AFTER_SCAN: OnceLock<Mutex<std::collections::BTreeSet<PathBuf>>> = OnceLock::new();
    FAIL_AFTER_SCAN.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()))
}

#[cfg(test)]
fn note_sync_leader_started(db_path: &Path) {
    let mut counts = sync_leader_counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *counts.entry(db_path.to_path_buf()).or_insert(0) += 1;
    drop(counts);

    if let Some(gate) = sync_leader_gate() {
        gate.mark_started();
        gate.wait_released();
    }
}

#[cfg(not(test))]
fn note_sync_leader_started(_db_path: &Path) {}

#[cfg(test)]
pub(crate) fn sync_leader_count(db_path: &Path) -> usize {
    sync_leader_counts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(db_path)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
fn sync_leader_counts() -> &'static Mutex<std::collections::BTreeMap<PathBuf, usize>> {
    static SYNC_LEADER_COUNTS: OnceLock<Mutex<std::collections::BTreeMap<PathBuf, usize>>> =
        OnceLock::new();
    SYNC_LEADER_COUNTS.get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
}

#[cfg(test)]
pub(crate) struct SyncLeaderGate {
    started: (Mutex<bool>, Condvar),
    release: (Mutex<bool>, Condvar),
}

#[cfg(test)]
impl SyncLeaderGate {
    pub(crate) fn new() -> Self {
        Self {
            started: (Mutex::new(false), Condvar::new()),
            release: (Mutex::new(false), Condvar::new()),
        }
    }

    fn mark_started(&self) {
        let (lock, ready) = &self.started;
        let mut started = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *started = true;
        ready.notify_all();
    }

    pub(crate) fn wait_started(&self, timeout: std::time::Duration) -> bool {
        let (lock, ready) = &self.started;
        let started = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (started, _) = ready
            .wait_timeout_while(started, timeout, |started| !*started)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *started
    }

    fn wait_released(&self) {
        let (lock, ready) = &self.release;
        let released = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _released = ready
            .wait_while(released, |released| !*released)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    pub(crate) fn release(&self) {
        let (lock, ready) = &self.release;
        let mut released = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *released = true;
        ready.notify_all();
    }
}

#[cfg(test)]
pub(crate) fn set_sync_leader_gate(gate: Option<Arc<SyncLeaderGate>>) {
    let mut slot = sync_leader_gate_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = gate;
}

#[cfg(test)]
pub(crate) fn set_sync_after_scan_gate(db_path: PathBuf, gate: Option<Arc<SyncLeaderGate>>) {
    let mut slot = sync_after_scan_gate_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = gate.map(|gate| (db_path, gate));
}

#[cfg(test)]
fn sync_leader_gate() -> Option<Arc<SyncLeaderGate>> {
    sync_leader_gate_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
fn sync_leader_gate_slot() -> &'static Mutex<Option<Arc<SyncLeaderGate>>> {
    static SYNC_LEADER_GATE: OnceLock<Mutex<Option<Arc<SyncLeaderGate>>>> = OnceLock::new();
    SYNC_LEADER_GATE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn sync_after_scan_gate(db_path: &Path) -> Option<Arc<SyncLeaderGate>> {
    sync_after_scan_gate_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|(gate_path, _gate)| gate_path == db_path)
        .map(|(_gate_path, gate)| Arc::clone(gate))
}

#[cfg(test)]
fn sync_after_scan_gate_slot() -> &'static Mutex<Option<(PathBuf, Arc<SyncLeaderGate>)>> {
    static SYNC_AFTER_SCAN_GATE: OnceLock<Mutex<Option<(PathBuf, Arc<SyncLeaderGate>)>>> =
        OnceLock::new();
    SYNC_AFTER_SCAN_GATE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
