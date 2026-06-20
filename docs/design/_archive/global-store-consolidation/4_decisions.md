---
title: Global Store Consolidation — Decisions
owner: codex
last_updated: 2026-05-23
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

**Status:** Accepted · 2026-05 · [ORB-00276]

**Context.** `.orbit/state/audit/v2_loop/`, `.orbit/state/job-runs/`, and `.orbit/state/sessions/<id>/learnings.json` are high-cardinality, machine-only stores. They create inode pressure and force scan-heavy queries for time ranges, denials, run status, and session resumption.

**Decision.** Store those records in `~/.orbit/orbit.db` with `workspace_id TEXT NOT NULL` sourced from `<workspace>/.orbit/config.yaml`. Release N imports legacy JSON once and then single-writes to SQLite; Release N+1 will delete the legacy JSON dirs and importer in a follow-up. Friction reports stay file-backed.

**Schema.**

```sql
CREATE TABLE IF NOT EXISTS v2_audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    source TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    ts TEXT NOT NULL,
    run_id TEXT NOT NULL,
    agent_identity TEXT NOT NULL,
    parent_event_id TEXT,
    workspace_path TEXT,
    payload_json TEXT NOT NULL,
    UNIQUE(workspace_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_ts ON v2_audit_events(workspace_id, ts);
CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_run ON v2_audit_events(workspace_id, run_id, ts);
CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_event_type ON v2_audit_events(workspace_id, event_type);

CREATE TABLE IF NOT EXISTS job_runs (
    run_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    state TEXT NOT NULL,
    scheduled_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    duration_ms INTEGER,
    created_at TEXT NOT NULL,
    pid INTEGER,
    pid_start_time TEXT,
    input_json TEXT,
    retry_source_run_id TEXT,
    knowledge_metrics_json TEXT,
    resolved_crew TEXT,
    planner_model TEXT,
    implementer_model TEXT,
    reviewer_model TEXT,
    pipeline_state_json TEXT,
    PRIMARY KEY(workspace_id, run_id)
);
CREATE INDEX IF NOT EXISTS idx_job_runs_ws_job_sched ON job_runs(workspace_id, job_id, scheduled_at DESC);
CREATE INDEX IF NOT EXISTS idx_job_runs_ws_state ON job_runs(workspace_id, state);

CREATE TABLE IF NOT EXISTS job_run_steps (
    workspace_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    step_index INTEGER NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    state TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    duration_ms INTEGER,
    exit_code INTEGER,
    error_code TEXT,
    error_message TEXT,
    agent_response_json TEXT,
    PRIMARY KEY(workspace_id, run_id, step_index),
    FOREIGN KEY(workspace_id, run_id) REFERENCES job_runs(workspace_id, run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS session_learning_state (
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    learning_injection_state_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(workspace_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_session_learning_state_ws ON session_learning_state(workspace_id, updated_at);

CREATE TABLE IF NOT EXISTS schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

**Consequences.**
- Reads default to the current workspace id; cross-workspace queries require a future explicit admin surface.
- `session_learning_state` is named specifically to avoid collision with existing `agent_sessions`.
- `audit/blobs/` remains content-addressed on disk; SQLite rows keep blob refs in `payload_json`.
- `.orbit/frictions/` remains file-backed because friction reports are human-edited, low-volume, and benefit from grep/diff/PR review.
- Cost: all workspaces share one WAL and busy-timeout envelope for higher-volume runtime writes.

## Task References

- ORB-00276 — accepted and implemented the first consolidation phase.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
