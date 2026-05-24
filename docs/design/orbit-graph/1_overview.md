---
summary: "Orbit Graph — Overview"
type: design
title: "Orbit Graph — Overview"
owner: claude
last_updated: 2026-05-24
status: Draft
feature: orbit-graph
doc_role: overview
tags: ["orbit-graph"]
related_features: [knowledge-graph]
---

# Orbit Graph — Overview

Orbit Graph is the proposed replacement for `orbit-knowledge`: a per-worktree SQLite-backed code index that agents can query for symbols, references, callees, impact, and command traces. Where `orbit-knowledge` is a content-addressed versioned store with mutable refs, locks, and a working-graph layer, Orbit Graph is a derived index — regenerable in seconds from `(file_contents, extractor_version)` — with no durable state beyond a single `.db` file per worktree.

This document is the entry point. The prescriptive V1 specification — schema, query surface, build pipeline, performance budgets, migration plan — lives in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) under `specs/`. [2_design.md](./2_design.md) is the long-form design discussion at a higher level of abstraction. [3_vision.md](./3_vision.md) captures the V2 write surface and other forward-looking items. [4_decisions.md](./4_decisions.md) is the ADR log (currently empty pending allocation via `orbit.adr.add`).

---

## 1. Motivation

The existing `orbit-knowledge` crate (~24k LOC) was designed as a git-like history layer: content-addressed JSON objects under `objects/<hh>/`, a SQLite sidecar for queries, mutable refs, locks for atomic swaps, and a `working_graph/` layer for staged edits. In practice the shape produced concrete failures:

1. **Two storage paths must agree.** Object store + SQLite sidecar drift; agents see stale or contradictory results.
2. **Unshipped mutation layer.** ~1.5k LOC of `working_graph/` exists but isn't exposed publicly, because the lock protocol cannot coordinate independent worktrees.
3. **Locks that don't lock the right thing.** Same-branch worktrees still race (acknowledged in `knowledge-graph` ADR-002).
4. **Full re-extraction on any file change.** No incremental refresh.
5. **Mixed concerns.** Query, mutation, durable storage, ref management, pack rendering, and task lineage share one crate.

The root cause: the graph was designed as a versioned store when the actual job is "fast, fresh, queryable index of the current code."

## 2. Core Concepts

**The graph is a derived index, not a source of truth.** Git owns the source files. The graph is reproducible from disk plus the extractor version. That reframe deletes the need for atomic ref swaps, object dedup, durable working graphs, and custom lock protocols — none of which were earning their complexity.

| Concept | Meaning |
|---|---|
| **Per-worktree DB** | One SQLite file per `(worktree, branch, extractor_version)`. Same-branch worktrees don't share state, so they can't corrupt each other. Disk cost ~10MB per worktree. |
| **Versioned by extractor** | The DB filename embeds the extractor version. When extractor logic changes, bump the version constant; old DBs become invisible and are deleted on next sync. No schema migrations to write. |
| **Two-pass build** | Pass 1 extracts symbols and imports in parallel; Pass 2 resolves cross-file refs by qualified name (not by symbol ID, which is ephemeral). |
| **Confidence as schema** | Every cross-file reference carries `exact`, `import_resolved`, `same_module`, or `fuzzy_name`. Agents filter by confidence floor at read time; the contract isn't a footnote. |
| **Structural only** | Tree-sitter is the backbone. No LSP, no rustc-as-a-library, no proc-macro expansion. Trait dispatch and dynamic calls degrade to `fuzzy_name` honestly. |

## 3. At a Glance

| Concern | File / module (planned) | Tracking task |
|---|---|---|
| Pure extraction per language | `crates/orbit-graph-extract/src/languages/` | (unscheduled) |
| ExtractedFile shape, Extractor trait | `crates/orbit-graph-extract/src/lib.rs` | (unscheduled) |
| SQLite schema, transactions | `crates/orbit-graph/src/store/` | (unscheduled) |
| Build pipeline, scanner, diff | `crates/orbit-graph/src/sync/` | (unscheduled) |
| Query API: search/show/refs/callees/impact/trace | `crates/orbit-graph/src/query/` | (unscheduled) |
| CLI subcommands + MCP wrappers | `crates/orbit-graph-cli/src/` | (unscheduled) |
| Selector parser (back-compat with skills) | `crates/orbit-graph-extract/src/selector.rs` | (unscheduled) |
| Equivalence harness for v1↔v2 dual-run | `tools/graph-equiv/` | (unscheduled) |

The four-step migration from `orbit-knowledge` to `orbit-graph` is laid out in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §16.

---

## Task References

No Orbit tasks have been allocated for this feature yet. Tasks will be created when the migration plan in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §16 enters execution.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
