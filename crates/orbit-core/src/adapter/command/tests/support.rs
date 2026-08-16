use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::OrbitRuntime;
use crate::adapter::command::dispatch::take_tool_audit_recorded;

/// Serializes tests that mutate `ORBIT_AGENT_*` or assert environment-derived
/// audit roles so parallel test execution cannot race environment writers.
pub(super) fn env_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn clear_identity_env() {
    // SAFETY: callers hold `env_guard()` while changing process environment.
    unsafe {
        std::env::remove_var("ORBIT_AGENT_NAME");
        std::env::remove_var("ORBIT_AGENT_MODEL");
    }
}

pub(super) fn set_identity_env(agent: &str, model: &str) {
    // SAFETY: callers hold `env_guard()` while changing process environment.
    unsafe {
        std::env::set_var("ORBIT_AGENT_NAME", agent);
        std::env::set_var("ORBIT_AGENT_MODEL", model);
    }
}

pub(super) fn fresh_runtime() -> OrbitRuntime {
    // Reset the dedup signal so cross-test thread-local leakage cannot mask
    // bugs in the per-call set/clear cycle.
    let _ = take_tool_audit_recorded();
    clear_identity_env();
    OrbitRuntime::in_memory().expect("build in-memory runtime")
}
