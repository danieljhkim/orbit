## Context
`backend: cli` captures stdout/stderr through reader threads while supervising a wall-clock timeout. Killing only the immediate child lets shell-spawned grandchildren survive, keep inherited pipe write ends open, and either hang reader joins or leak background work after a timed-out activity.

## Decision
Spawn bare Unix CLI subprocesses as process-group leaders, matching the existing macOS sandbox wrapper boundary. On timeout, signal the whole child process group with `SIGKILL`, wait for the main child, and bound timeout-path reader joins; after a normal child exit, clean up the same process group before joining readers so orphaned pipe holders do not block capture.

## Consequences
- CLI subprocess supervision has one Unix tree boundary for bare and macOS-sandboxed paths.
- Output capture still preserves partial stdout/stderr bytes already drained before timeout, even if a reader thread does not finish within the bounded join window.
- Cost: Unix process groups do not cover descendants that deliberately create a new session/process group, and non-Unix platforms still use the immediate-child fallback until an equivalent tree-kill primitive is added.
