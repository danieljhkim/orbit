---
title: Global Store Consolidation — Decisions
owner: codex
last_updated: 2026-08-11
status: Accepted
feature: global-store-consolidation
doc_role: decisions
type: design
summary: ADR log for consolidating v2 runtime state into the global SQLite store.
tags: [global-store-consolidation, storage, sqlite]
paths: ["crates/orbit-store/**", "crates/orbit-core/**", "crates/orbit-engine/**", "crates/orbit-dashboard/**"]
related_features: [global-store-consolidation]
related_artifacts: [ORB-00276, ADR-0183]
---

# Global Store Consolidation — Decisions

ADR entries use globally allocated `ADR-NNNN` identifiers. Metadata lives in `.orbit/adrs/`; this file is the local narrative log.

## ADR-0183 — Consolidate v2 audit, job-run, and session-learning state into the global SQLite store

**Status:** Superseded by ADR-0344 · 2026-05-23 05:40:56.163428Z · [ORB-00276]
**Owner:** codex
**Created:** 2026-05-23 05:40:51.837400Z
**Last updated:** 2026-08-09 19:32:05.196900+00:00
**Related features:** `global-store-consolidation`
**Tags:** `storage`, `sqlite`, `migration`
**Paths:** `crates/orbit-store/**`, `crates/orbit-core/**`, `crates/orbit-engine/**`, `crates/orbit-agent/**`, `crates/orbit-dashboard/**`, `docs/design/global-store-consolidation/**`

### Context
Orbit currently writes high-cardinality v2 audit envelopes, job-run records, and per-session learning admission state as JSON files under each workspace. Those stores are machine-only, grow quickly, and need indexed time/status lookups, while the global SQLite store already owns append-oriented machine data.

### Decision
Consolidate the v2 audit envelope rows, job run/step rows, and session learning state rows into `~/.orbit/orbit.db`, keyed by the workspace identity from `<workspace>/.orbit/config.yaml` `workspace_id`. Release N performs a one-shot import and then single-writes new rows to SQLite while leaving legacy JSON directories in place; Release N+1 will remove the legacy directories and importer. Friction reports and audit blob bodies remain file-backed.

### Consequences
- V2 audit, job-run, and session-learning reads can use indexed SQLite queries scoped by `workspace_id`; cross-workspace queries remain a future explicit admin surface.
- Table names are explicit: `v2_audit_events`, `job_runs`, `job_run_steps`, and `session_learning_state`; the last avoids confusion with the existing `agent_sessions` table.
- Legacy JSON directories are retained as read-only fallback evidence for one release; import idempotency is tracked in `schema_meta` by workspace.
- Friction reports under `.orbit/frictions/`, diagnostics, logs, scoreboards, worktrees, and content-addressed audit blobs stay on disk.
- Cost: The global SQLite database now carries higher write volume from all workspaces, so callers must preserve workspace scoping and rely on WAL/busy-timeout behavior rather than per-workspace file isolation.

## ADR-0344 — Include friction records in the global SQLite coordination store

**Status:** Accepted · 2026-08-09 19:30:16.031389Z · [ORB-10680]
**Owner:** codex
**Created:** 2026-08-09 19:30:02.306007Z
**Last updated:** 2026-08-09 19:30:16.031389Z
**Related features:** `global-store-consolidation`, `friction`, `host-registry`
**Supersedes:** `ADR-0183`
**Tags:** `storage`, `sqlite`, `migration`, `performance`
**Paths:** `crates/orbit-store/src/sqlite/**`, `crates/orbit-store/src/file/friction_store/**`, `crates/orbit-core/src/runtime/orbit_tool_host/**`, `docs/design/mcp-bridge/**`, `docs/design/auditability/**`

### Context
ADR-0183 consolidated high-cardinality machine state into `~/.orbit/orbit.db` but explicitly left frictions file-backed on the assumption that they were low-volume, human-edited records benefiting from grep, diff, and PR review. The hub coordination model has since made frictions hub-only and tool-mutated, while the file store performs corpus-wide discovery, YAML parsing, allocation, filtering, sorting, and aggregation for narrow reads. The alternatives are to maintain a separate file database with its own locks and migration markers or extend the already-ledgered global SQLite coordination store.

### Decision
Extend the global SQLite coordination store to own live friction records, tags, workspace-month allocation counters, and per-workspace import state. A ledgered schema migration creates the tables and indexes; workspace activation transactionally imports canonical hub and legacy checkout records, verifies complete field/content preservation, and commits an idempotent import marker. After that marker, Orbit single-writes and reads SQLite only. Existing v2 audit, job-run, session-learning, blob, diagnostic, log, scoreboard, and worktree placement decisions remain unchanged. This is a scan-memory and query-shaping change, not a retention or disk-reclamation policy.

### Consequences
- Workspace, status, time, model, tag, ordering, pagination, and stats queries execute against indexed rows without first materializing every body.
- Composite `(workspace_id, friction_id)` identity preserves workspace-local IDs and prevents cross-workspace collisions.
- Legacy friction files remain untouched as read-only evidence for one release; repeated or interrupted imports are safe, and malformed or conflicting sources fail before authority switches.
- SQLite integrity checks, backups, WAL/busy-timeout behavior, and the newer-schema downgrade guard cover frictions alongside existing coordination state.
- Cost: The global database gains another write domain and a per-workspace data migration; rollback to a pre-migration binary requires the guarded export/recovery procedure rather than silently resuming writes to stale files.

## Task References

- ORB-00276 — accepted and implemented the first consolidation phase.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
