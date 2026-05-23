---
title: Global Store Consolidation — Design
owner: codex
last_updated: 2026-05-23
status: Draft
feature: global-store-consolidation
doc_role: design
type: design
summary: Current implementation of SQLite-backed v2 audit, job-run, and session-learning stores.
tags: [global-store-consolidation, storage, sqlite]
paths: ["crates/orbit-store/**", "crates/orbit-core/**", "crates/orbit-engine/**"]
related_features: [global-store-consolidation]
related_artifacts: [ORB-00276, ADR-0183]
---

# Global Store Consolidation — Design

Scope is limited to high-cardinality machine stores: v2 audit events, job runs and steps, and session learning admission state.

## 1. Schema Ownership

`orbit-store` owns schema creation in `crates/orbit-store/src/sqlite/migration/mod.rs`. Each migrated table includes `workspace_id TEXT NOT NULL` and workspace-scoped indexes. `schema_meta` records the one-shot import marker.

## 2. Store APIs

Typed SQLite params and rows live beside the SQL queries. Job-run callers continue to use `JobRunStoreBackend`; the factory returns the SQLite implementation once the runtime resolves the workspace id.

## 3. Import

`state_io::import::import_legacy_v2_state` reads legacy JSONL/YAML/JSON state from the current workspace and inserts deterministic SQLite rows. It sets `schema_meta` only after the import pass completes without skipped audit lines.

## 4. Concerns & Honest Limitations

Archive semantics are represented through the active SQLite row set rather than an archived directory tree. A future admin surface can add explicit archive metadata if operators need to inspect archived rows separately from deletion.

## Task References

- ORB-00276 — implemented the first SQLite-backed consolidation pass.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
