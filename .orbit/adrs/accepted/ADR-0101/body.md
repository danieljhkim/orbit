## Context
Installing parent-side SIGINT/SIGTERM handlers is a process-global operation. Two concurrent `run_process` calls cannot install independent handlers without races, and a panicking call must restore the prior handler so the orbit process itself remains interruptible.

## Decision
`SignalHandlerGuard::install` acquires a `Mutex` from a `OnceLock`, creates a non-blocking pipe, calls `libc::sigaction` for SIGINT and SIGTERM, and stores the previous `sigaction` structs. Drop reverses the steps: restore previous handlers, close the pipe, release the mutex. The handler itself is async-signal-safe (atomic load + 1-byte `write`).

## Consequences
- Concurrent `run_process` calls serialize handler install/drop, and panics still restore prior handlers via Drop.
- Cost: contention on the global mutex limits exec parallelism in a single process. Named as an open question in [3_vision.md §1.11](./3_vision.md#1-open-questions).
