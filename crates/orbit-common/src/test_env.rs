//! Ambient-process isolation for tests — environment variables and umask.
//!
//! Both halves of this module exist for the same reason: a test that depends
//! on ambient process state passes or fails according to *how the suite was
//! launched* rather than what the code does. See [`unset`] for inherited env
//! vars and [`harden_dir`] for umask-derived directory permissions.
//!
//! Several Orbit surfaces read agent identity and run context from the
//! process environment — notably runtime actor identity and `tool run`
//! audit-role resolution. A test that asserts the *absence* of that context
//! ("attributes to the human actor", "falls back to the agent role") is only
//! correct when those variables are genuinely unset.
//!
//! GitHub CI runs the suite from a bare shell, so the assertions hold there by
//! accident. An agent running the same suite from inside a managed Orbit run
//! inherits `ORBIT_RUN_ID`, `ORBIT_AGENT_MODEL`, `ORBIT_TASK_ID`, … and those
//! tests flip to red for a reason that has nothing to do with the code under
//! test (ORB-10350). [`unset`] makes the expectation explicit instead of
//! ambient.
//!
//! Exposed behind the `test-util` feature so integration tests and sibling
//! crates share one implementation rather than re-deriving the guard.

use std::sync::{Mutex, MutexGuard, OnceLock};

/// The identity pair consulted when a command carries no explicit
/// `--agent`/`--model` and no input attribution.
pub const AGENT_IDENTITY_ENV: &[&str] = &["ORBIT_AGENT_NAME", "ORBIT_AGENT_MODEL"];

/// Restores the variables captured by [`unset`] when dropped.
///
/// Holds a process-wide lock for its lifetime, so concurrent tests cannot
/// interleave environment mutations. Keep the guard's scope tight — in
/// particular, drop it before an `.await` (`clippy::await_holding_lock`); the
/// env is typically read during synchronous construction, so binding it around
/// just that call is enough.
#[must_use = "the environment is restored as soon as the guard is dropped"]
pub struct ScopedEnv {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<String>)>,
}

/// Clear `names` for the returned guard's lifetime, restoring prior values on
/// drop. Names that are already unset are restored as unset.
pub fn unset<'a>(names: impl IntoIterator<Item = &'a str>) -> ScopedEnv {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let saved = names
        .into_iter()
        .map(|name| (name.to_string(), std::env::var(name).ok()))
        .collect::<Vec<_>>();
    // SAFETY: the guard holds the process-wide lock for the whole mutation
    // window, so no other guarded reader/writer runs concurrently.
    unsafe {
        for (name, _) in &saved {
            std::env::remove_var(name);
        }
    }
    ScopedEnv { _lock: lock, saved }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        // SAFETY: the guard still holds the process-wide lock here.
        unsafe {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// Pin `path` to `0o700`, making a fixture directory independent of the
/// ambient umask.
///
/// `tempfile::tempdir()` creates its root with `0o777 & !umask`. CI runs with
/// the conventional `umask 022`, so the root lands `0o755` and every
/// permission-sensitive check downstream happens to pass. A developer box with
/// a permissive umask (`002`, common with user-private groups, or `000`) gets
/// a group- or world-writable root instead — and Orbit's search-companion
/// override validator legitimately refuses to execute a binary whose parent
/// directory is group/world writable. The fixture, not the validator, is what
/// needs to be deterministic (ORB-10350).
///
/// No-op on non-Unix targets.
#[cfg(unix)]
pub fn harden_dir(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("read fixture dir metadata at {}: {error}", path.display()));
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)
        .unwrap_or_else(|error| panic!("chmod fixture dir {}: {error}", path.display()));
}

/// Non-Unix targets have no umask to defend against.
#[cfg(not(unix))]
pub fn harden_dir(_path: &std::path::Path) {}
