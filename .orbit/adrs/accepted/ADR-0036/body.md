## Context
The unified JSONL feed still lacked job-DAG lifecycle projections, library print hygiene, and a first-class reader for the v2-terminal-console mockup.

## Decision
Add one `emit_job_event` dual-write helper for job lifecycle tracing, migrate library `println!`/`eprintln!` calls to structured tracing with clippy denies in library crates, and add `orbit log tail` with path, target, level, since, follow, and JSON options.

## Consequences
- The terminal-console mockup can use real Orbit events, and library crates fail clippy if raw prints return.
- Cost: scheduler-event semantics remain aspirational, follow mode is v1, and the reader keeps the file in memory before applying `-n`.
