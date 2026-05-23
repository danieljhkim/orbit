## Context
Orbit currently writes high-cardinality v2 audit envelopes, job-run records, and per-session learning admission state as JSON files under each workspace. Those stores are machine-only, grow quickly, and need indexed time/status lookups, while the global SQLite store already owns append-oriented machine data.

## Decision
Consolidate the v2 audit envelope rows, job run/step rows, and session learning state rows into `~/.orbit/orbit.db`, keyed by the workspace identity from `<workspace>/.orbit/config.yaml` `workspace_id`. Release N performs a one-shot import and then single-writes new rows to SQLite while leaving legacy JSON directories in place; Release N+1 will remove the legacy directories and importer. Friction reports and audit blob bodies remain file-backed.

## Consequences
- V2 audit, job-run, and session-learning reads can use indexed SQLite queries scoped by `workspace_id`; cross-workspace queries remain a future explicit admin surface.
- Table names are explicit: `v2_audit_events`, `job_runs`, `job_run_steps`, and `session_learning_state`; the last avoids confusion with the existing `agent_sessions` table.
- Legacy JSON directories are retained as read-only fallback evidence for one release; import idempotency is tracked in `schema_meta` by workspace.
- Friction reports under `.orbit/frictions/`, diagnostics, logs, scoreboards, worktrees, and content-addressed audit blobs stay on disk.
- Cost: The global SQLite database now carries higher write volume from all workspaces, so callers must preserve workspace scoping and rely on WAL/busy-timeout behavior rather than per-workspace file isolation.