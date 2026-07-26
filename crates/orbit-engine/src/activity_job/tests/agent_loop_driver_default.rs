#![allow(missing_docs)]

use super::super::agent_loop_driver::replay_active;

struct ReplayEnvGuard {
    replay: Option<String>,
    fixture: Option<String>,
}

impl ReplayEnvGuard {
    fn set_both() -> Self {
        let guard = Self {
            replay: std::env::var("ORBIT_V2_REPLAY").ok(),
            fixture: std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok(),
        };
        // SAFETY: this test restores both variables before returning. The
        // default build's predicate returns before reading process state.
        unsafe {
            std::env::set_var("ORBIT_V2_REPLAY", "tool_denial");
            std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", "/tmp/inert-replay-fixture");
        }
        guard
    }
}

impl Drop for ReplayEnvGuard {
    fn drop(&mut self) {
        // SAFETY: restore the process environment captured by `set_both`.
        unsafe {
            match &self.replay {
                Some(value) => std::env::set_var("ORBIT_V2_REPLAY", value),
                None => std::env::remove_var("ORBIT_V2_REPLAY"),
            }
            match &self.fixture {
                Some(value) => std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", value),
                None => std::env::remove_var("ORBIT_V2_REPLAY_FIXTURE"),
            }
        }
    }
}

#[test]
fn replay_environment_is_inert_without_feature() {
    let _guard = ReplayEnvGuard::set_both();
    assert!(!replay_active());
}
