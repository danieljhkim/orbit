---
title: Global Store Consolidation — Overview
owner: codex
last_updated: 2026-06-20
status: Accepted
feature: global-store-consolidation
doc_role: overview
type: design
summary: Consolidates high-cardinality v2 runtime state into the global SQLite store with workspace_id scoping.
tags: [global-store-consolidation, storage, sqlite]
paths: ["crates/orbit-store/**", "crates/orbit-core/**", "crates/orbit-engine/**"]
related_features: [global-store-consolidation]
related_artifacts: [ORB-00276, ADR-0183]
---

# Global Store Consolidation — Overview

> **Archived (2026-06) — completed feature, historical record.** The consolidation described here shipped under [ORB-00276]: v2 audit, job-run, and session-learning state now live in the global SQLite store (`crates/orbit-store/src/sqlite/`). This folder was moved out of the active design set during a docs-cleanup pass; it is retained as the design record behind **ADR-0183** (still allocated in the store). No further design work is tracked here.

High-cardinality, machine-only runtime stores now use the existing global SQLite database instead of JSON-per-record workspace files.

## 1. Motivation

The v2 audit envelope, job-run lifecycle, and session-learning stores grow quickly and need indexed lookup by workspace, run, state, event type, and time. Keeping those stores as JSON files under every workspace creates inode pressure and slow scans without giving humans useful review material.

## 2. Core Concepts

| Concept | Meaning |
|---|---|
| Global DB | `~/.orbit/orbit.db`, opened through `orbit-store::Store`. |
| Workspace discriminator | `workspace_id` from `<workspace>/.orbit/config.yaml`. |
| One-shot import | Release N imports legacy JSON once and leaves the old dirs intact. |
| File-backed boundary | Friction reports and audit blobs stay on disk. |

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| SQLite schema | `crates/orbit-store/src/sqlite/migration/mod.rs` | ORB-00276 |
| Store APIs | `crates/orbit-store/src/sqlite/*_store/mod.rs` | ORB-00276 |
| Runtime bootstrap import | `crates/orbit-core/src/runtime/builder.rs` | ORB-00276 |
| Decision | `docs/design/_archive/global-store-consolidation/4_decisions.md` | ORB-00276 |

## Task References

- ORB-00276 — migrated v2 audit, job-run, and session-learning state to SQLite.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
