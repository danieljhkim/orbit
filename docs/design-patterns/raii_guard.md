---
type: pattern
summary: "RAII Guard Pattern"
---
# RAII Guard Pattern

Bind a side effect to a lexical scope: do something at construction, undo it in `Drop`. Callers write `let _g = Guard::enter(...);` and rely on scope exit — including `?`-return and panic unwind — to clean up. The defining trait: **`Drop` does meaningful work** (restoring state, releasing a lock, persisting a record), not just freeing memory.

```rust
struct Guard { /* captured state to undo */ }

impl Guard {
    fn enter(...) -> Self { /* install / acquire / stage */ }
}

impl Drop for Guard {
    fn drop(&mut self) { /* restore / release / finalize */ }
}
```

Four shapes in the codebase carry distinct lessons.

## Reference: `AuditGuard` — record the scope's outcome once

From `crates/orbit-cli/src/audit_middleware.rs:39`:

```rust
pub struct AuditGuard<'a> {
    runtime: &'a OrbitRuntime,
    meta: CommandMeta,
    start: Instant,
    status: AuditEventStatus,    // defaults to Failure
    exit_code: i32,              // defaults to -1
    error_message: Option<String>,
}

impl AuditGuard<'_> {
    pub fn mark_success(&mut self) { /* ... */ }
    pub fn mark_failure(&mut self, error: &OrbitError) { /* ... */ }
}

impl Drop for AuditGuard<'_> {
    fn drop(&mut self) {
        if take_tool_audit_recorded() { return; }       // suppression flag
        let params = AuditEventInsertParams { /* ... */ };
        let write = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || self.runtime.record_audit_event(&params),
        ));
        // log and swallow; never propagate from Drop
    }
}
```

Patterns to copy:

- **Default to the "bad" outcome.** Status starts as `Failure`/`-1`; scope exits without an explicit `mark_*` correctly reflect "process died mid-command."
- **Mutation methods on `&mut self`, not constructor params.** Caller updates the outcome as it learns; `Drop` reads final state.
- **`catch_unwind` around the side effect.** A panic *during audit emission* can't double-panic the unwind.

## Reference: `StagedTextFile` — `Drop` as rollback

From `crates/orbit-common/src/utility/fs.rs:74`:

```rust
pub struct StagedTextFile {
    target_path: PathBuf,
    temp_path: PathBuf,
    committed: bool,
}

impl StagedTextFile {
    pub fn new(target: &Path, content: &str) -> io::Result<Self> { /* write temp file */ }
    pub fn commit(&mut self) -> io::Result<()> { /* rename, set committed = true */ }
}

impl Drop for StagedTextFile {
    fn drop(&mut self) {
        if self.committed { return; }
        let _ = fs::remove_file(&self.temp_path);
    }
}
```

Patterns to copy:

- **`committed: bool` is the lever.** Caller explicitly opts into the success path by calling `commit()`. Drop = rollback by default.
- **Shape for "stage → validate → commit-or-bail."** Between `new()` and `commit()`, the caller can inspect the staged content; any early-return cleans up the temp file automatically.

## Reference: `SignalHandlerGuard` — restore global state

From `crates/orbit-exec/src/supervision/signal.rs:9`:

```rust
pub(super) struct SignalHandlerGuard {
    previous_sigint: libc::sigaction,
    previous_sigterm: libc::sigaction,
    read_fd: i32,
    write_fd: i32,
    _lock: MutexGuard<'static, ()>,   // serialize installs across the process
}

impl SignalHandlerGuard {
    pub(super) fn install() -> Result<Self, OrbitError> {
        let lock = SIGNAL_HANDLER_LOCK.get_or_init(|| Mutex::new(())).lock()?;
        let (read_fd, write_fd) = create_signal_pipe()?;
        SIGNAL_PIPE_WRITE_FD.store(write_fd, Ordering::SeqCst);
        let previous_sigint = install_signal_handler(libc::SIGINT)?;
        let previous_sigterm = match install_signal_handler(libc::SIGTERM) {
            Ok(prev) => prev,
            Err(err) => {
                // hand-rollback: SIGINT installed but SIGTERM failed, and Drop
                // never runs on a value that never returned from this function
                SIGNAL_PIPE_WRITE_FD.store(-1, Ordering::SeqCst);
                close_fd(read_fd); close_fd(write_fd);
                restore_signal_handler(libc::SIGINT, &previous_sigint);
                return Err(err);
            }
        };
        Ok(Self { previous_sigint, previous_sigterm, read_fd, write_fd, _lock: lock })
    }
}

impl Drop for SignalHandlerGuard {
    fn drop(&mut self) {
        SIGNAL_PIPE_WRITE_FD.store(-1, Ordering::SeqCst);
        restore_signal_handler(libc::SIGINT, &self.previous_sigint);
        restore_signal_handler(libc::SIGTERM, &self.previous_sigterm);
        close_fd(self.read_fd);
        close_fd(self.write_fd);
    }
}
```

Patterns to copy:

- **Capture prior state in the guard's fields.** `previous_sigint` isn't recomputed in `Drop` — it's snapshotted at install time.
- **Hold a `'static` mutex as a field.** `_lock: MutexGuard<'static, ()>` makes "one guard at a time, process-wide" structurally impossible to violate.
- **Hand-rollback on partial install.** `Drop` only runs on values that successfully return; mid-construction failures must unwind their own work before returning `Err`.

## Reference: `GraphLockGuard` — explicit release with `Drop` fallback

From `crates/orbit-knowledge/src/lock.rs:209`:

```rust
pub struct GraphLockGuard {
    /* ...selectors, owner, paths... */
    released: bool,
}

impl GraphLockGuard {
    pub fn release(&mut self) -> Result<(), KnowledgeError> {
        if self.released { return Ok(()); }
        /* ...unlock each selector, persist store... */
        self.released = true;
        Ok(())
    }
}

impl Drop for GraphLockGuard {
    fn drop(&mut self) {
        if self.released { return; }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Err(error) = self.release() {
                tracing::warn!(target: "orbit.knowledge.lock", error = %error, "...");
            }
        }));
    }
}
```

Patterns to copy:

- **`released: bool` for idempotency.** `release()` followed by scope exit is a no-op, not a double-release.
- **Explicit `release()` returns `Result`; `Drop` only logs.** Callers that can react to a release error get a real error path. Callers that can't (or whose scope just ended) get the Drop fallback.

---

**Note on test fixtures.** Several test files use small `EnvVarGuard` / `TempDir` structs that save-and-restore env vars or `remove_dir_all` on drop. They follow the pattern but are too thin to be reference-grade; lift from a production guard above when writing new ones.
