---
summary: "Orbit Graph — Overview"
type: design
title: "Orbit Graph — Overview"
owner: claude
last_updated: 2026-07-25
status: Draft
feature: orbit-graph
doc_role: overview
tags: ["orbit-graph"]
paths: ["crates/orbit-graph/**"]
related_features: [knowledge-graph]
related_artifacts: [ORB-00391, ORB-00396, ORB-10011, ORB-10225, ORB-10319, ORB-10325, ORB-10357]
---

# Orbit Graph — Overview

Orbit Graph is the code-intelligence layer for Orbit: a per-worktree SQLite-backed code index that agents can query for symbols, references, callees, impact, and command traces. It is a derived index — regenerable in seconds from `(file_contents, extractor_version)` — with no durable state beyond a single `.db` file per worktree. It replaced the former `orbit-knowledge` crate, a content-addressed versioned store with mutable refs, locks, and a working-graph layer.

The v2 cutover completed in ORB-00391: `orbit-knowledge` (v1) was decommissioned and orbit-graph is now the sole graph implementation. Its external surface was the CLI-only `orbit graph` command embedded by `orbit-cli`; ORB-10325 and [orbit-graph is CLI-surface only; separate MCP deferred until a shell-less consumer exists](./4_decisions.md#orbit-graph-is-cli-surface-only-separate-mcp-deferred-until-a-shell-less-consumer-exists) had already removed graph from MCP and from the registered `orbit.*` tool namespace. **ORB-10357 removed the `orbit graph` subcommand from `orbit-cli` entirely and folded the former `orbit-graph-extract` and `orbit-graph-cli` crates into `orbit-graph` as the `extract` and `cli` modules.** `orbit-graph` now has **zero workspace dependents** and no command surface of any kind (CLI, MCP, or tool registry); it is consolidated into a single crate and parked awaiting deletion — no further investment planned. See [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §16 for the migration outcome.

This document is the entry point. The prescriptive V1 specification — schema, query surface, build pipeline, performance budgets, migration plan — lives in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) under `specs/`. [2_design.md](./2_design.md) is the long-form design discussion at a higher level of abstraction. [3_vision.md](./3_vision.md) captures the V2 write surface and other forward-looking items. [4_decisions.md](./4_decisions.md) is the ADR log; the v2 cutover + decommission is recorded in [Cut over to orbit-graph (v2) and decommission orbit-knowledge](./4_decisions.md#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge) (supersedes [Roll back orbit-graph tool cutover to orbit-knowledge](./4_decisions.md#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)).

---

## 1. Motivation

The existing `orbit-knowledge` crate (~24k LOC) was designed as a git-like history layer: content-addressed JSON objects under `objects/<hh>/`, a SQLite sidecar for queries, mutable refs, locks for atomic swaps, and a `working_graph/` layer for staged edits. In practice the shape produced concrete failures:

1. **Two storage paths must agree.** Object store + SQLite sidecar drift; agents see stale or contradictory results.
2. **Unshipped mutation layer.** ~1.5k LOC of `working_graph/` exists but isn't exposed publicly, because the lock protocol cannot coordinate independent worktrees.
3. **Locks that don't lock the right thing.** Same-branch worktrees still race (acknowledged in [Branch-scoped refs over a single shared ref](../knowledge-graph/4_decisions.md#branch-scoped-refs-over-a-single-shared-ref)).
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

| Concern | File / module | Tracking task |
|---|---|---|
| Pure extraction per language | `crates/orbit-graph/src/extract/languages/` | (unscheduled) |
| ExtractedFile shape, Extractor trait | `crates/orbit-graph/src/extract/mod.rs` | (unscheduled) |
| SQLite schema, transactions | `crates/orbit-graph/src/store/` | (unscheduled) |
| Build pipeline, scanner, diff | `crates/orbit-graph/src/sync/` | (unscheduled) |
| Query API: search/show/refs/callees/impact/trace | `crates/orbit-graph/src/query/` | (unscheduled) |
| Former human/script CLI subcommands (dependent-free, parked) | `crates/orbit-graph/src/cli/` | [ORB-00396], [ORB-10357] |
| Canonical selector parser (re-exported by extract) | `crates/orbit-common/src/utility/selector.rs` | [ORB-10011] |

The four-step migration plan — landing orbit-graph alongside orbit-knowledge, dual-running them through an equivalence harness, then measuring effectiveness head-to-head before any phase-out decision — is laid out in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §16.

---

## Task References

The orbit-graph design and migration shipped across ORB-00294, ORB-00331, ORB-00344, ORB-00377, ORB-00385, and ORB-00391 (the v2 cutover + orbit-knowledge decommission). ORB-10319 temporarily moved agent MCP composition into the vertical Remote feature crate; ORB-10325 removed that surface and retained graph exclusively in the CLI. ORB-10357 then removed the CLI surface too and consolidated the three graph crates into one, leaving `orbit-graph` dependent-free. The per-ADR task mapping is in [4_decisions.md](./4_decisions.md) § Task References.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
