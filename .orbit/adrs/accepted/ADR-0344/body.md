## Context
ADR-0183 consolidated high-cardinality machine state into `~/.orbit/orbit.db` but explicitly left frictions file-backed on the assumption that they were low-volume, human-edited records benefiting from grep, diff, and PR review. The hub coordination model has since made frictions hub-only and tool-mutated, while the file store performs corpus-wide discovery, YAML parsing, allocation, filtering, sorting, and aggregation for narrow reads. The alternatives are to maintain a separate file database with its own locks and migration markers or extend the already-ledgered global SQLite coordination store.

## Decision
Extend the global SQLite coordination store to own live friction records, tags, workspace-month allocation counters, and per-workspace import state. A ledgered schema migration creates the tables and indexes; workspace activation transactionally imports canonical hub and legacy checkout records, verifies complete field/content preservation, and commits an idempotent import marker. After that marker, Orbit single-writes and reads SQLite only. Existing v2 audit, job-run, session-learning, blob, diagnostic, log, scoreboard, and worktree placement decisions remain unchanged. This is a scan-memory and query-shaping change, not a retention or disk-reclamation policy.

## Consequences
- Workspace, status, time, model, tag, ordering, pagination, and stats queries execute against indexed rows without first materializing every body.
- Composite `(workspace_id, friction_id)` identity preserves workspace-local IDs and prevents cross-workspace collisions.
- Legacy friction files remain untouched as read-only evidence for one release; repeated or interrupted imports are safe, and malformed or conflicting sources fail before authority switches.
- SQLite integrity checks, backups, WAL/busy-timeout behavior, and the newer-schema downgrade guard cover frictions alongside existing coordination state.
- Cost: The global database gains another write domain and a per-workspace data migration; rollback to a pre-migration binary requires the guarded export/recovery procedure rather than silently resuming writes to stale files.