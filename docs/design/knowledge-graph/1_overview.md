# Knowledge Graph — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-10

The knowledge graph is Orbit's durable, queryable codebase map: a content-addressed, branch-scoped tree of directories, files, and extracted symbols. It sits between raw files and the agent prompt so agents can ask *"where is `AgentRuntime` defined?"* without re-reading the repo from scratch. Task attribution was removed in [T20260506-11]; task IDs now remain local commit-search keys rather than graph fields. This compaction pass [T20260430-22] keeps the overview focused on entry-point concepts and leaves mechanism detail to [2_design.md](./2_design.md).

This document is the entry point. [2_design.md](./2_design.md) specifies the current implementation in detail; [3_vision.md](./3_vision.md) captures open questions and direction.

---

## 1. Motivation

Unassisted agents mostly navigate with `grep`, `find`, and full-file reads. Each has a sharp edge:

1. **Grep is string-level.** It mixes call sites, comments, fields, and unrelated names.
2. **Full-file reads bloat context.** A 30-line answer often arrives wrapped in hundreds of irrelevant lines.
3. **Cold sessions forget structure.** Module layout, trait hierarchies, and import graphs get re-derived every turn.

An LSP solves #1 and parts of #2 but is hostile to agent loops: it is stateful, process-local, and returns results tuned for IDE UX (hover text, rename previews) rather than for token-efficient prompt assembly.

The graph precomputes that structure once per branch, persists it, and exposes prompt-shaped query tools.

---

## 2. Core Concepts

### 2.1 Node types

The graph has three node kinds:

- **`DirNode`** — a directory with child dirs/files; the root id derives from `"."`.
- **`FileNode`** — a source file with extracted leaves plus a `source_blob_hash`.
- **`LeafNode`** — an extracted symbol or doc/config/table leaf with source span, hash, signature fields, and history.

All three share `BaseNodeFields`: `id`, `identity_key`, `location`, `language`, `description`, `parent_id`, and lock state. Historical `task_ids` attribution fields were removed in [T20260506-11].

The rename from `leaf` to `symbol` at the tool surface happened under [T20260411-0424]; the in-code type is still `LeafNode` because the internal vocabulary predates that decision and the rename has not yet reached the type name.

### 2.2 Identity vs. hash

Every node has three independent keys with different stability properties:

| Key | Role | Stability |
|-----|------|-----------|
| `id` | Primary reference within a graph snapshot | Stable across rebuilds of the same repo state |
| `identity_key` | Cross-build lineage used by the working graph to track renames and re-identification | Stable across rebuilds |
| `object_hash` | Content hash of the serialized node | Changes whenever any field changes |

Ids are derived from kind + location + discriminator. Object hashes drive deduplication in the object store.

### 2.3 Refs, indexes, and objects

Storage follows a git-style split:

```
.orbit/knowledge/graph/
├── objects/<hh>/<hash>.json     immutable, content-addressed node bodies
├── blobs/<hh>/<hash>.txt        immutable, content-addressed file/symbol source
├── index/by-id/<root-graph-hash>.json   immutable per-build index
├── graph_index.sqlite           mutable secondary index for current fast reads
└── refs/heads/<branch>.json     mutable branch ref → active index
```

A rebuild writes new immutable objects/blobs and index, refreshes the SQLite sidecar for current fast reads, then atomically swings the branch ref. Refs are the only authoritative mutable pointer, so concurrent builds on different worktrees cannot corrupt each other ([T20260421-0358], [T20260509-70], [T20260509-72]). See [specs/refs.md](./specs/refs.md).

### 2.4 Working graph and write guards

The current public graph surface is read-only. Agents use it to inspect, search, and pack context, while write coordination happens before dispatch in `task_gate_pipeline`: its `reserve_locks` activity reserves task `context_files` as a preflight guard. That guard operates at the workspace/task plane rather than inside a branch-local graph ref, which keeps it meaningful when agents work in separate worktrees.

The in-memory **working graph** (`crates/orbit-knowledge/src/working_graph`) remains an internal/deferred mutation substrate. Public graph write tools are absent because branch-local graph locks do not coordinate independent worktrees.

### 2.5 Task IDs

The graph no longer stores task attribution. `[T...]` commit tags remain useful for local forward lookup with `git log --grep`, but reverse lookup from graph node to task was removed in [T20260506-11] after a 10-day audit found 0 uses across 961 graph tool calls.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Crate boundary | `crates/orbit-knowledge` | [T20260411-0008], [T20260411-0424] |
| Storage layout | `src/graph/object_store.rs`, `src/graph/sqlite_index.rs` | [T20260421-0358], [T20260509-70] |
| Build pipeline | `src/pipeline/` | [T20260411-0424], [T20260417-0639], [T20260426-0139], [T20260509-33] |
| Historical task attribution removal | `src/pipeline/` | [T20260506-11] |
| Query commands | `src/commands/` with lower-level `src/service/` helpers | [T20260412-0645-2], [T20260412-0645-3], [T20260509-72], [T20260510-5] |
| Working graph | `src/working_graph/` | [T20260411-0424] |
| Locking | `src/lock.rs` | [T20260411-0424], [T20260417-0301-2] |
| Refresh safety | `src/pipeline/mod.rs` | [T20260417-0307], [T20260416-0719] |

---

## Task References

- **[T20260411-0008]** — Extract `orbit-knowledge` crate from `orbit-tools`.
- **[T20260411-0424]** — Consolidate `orbit-knowledge`; prototype graph editing internals (`add`/`delete`/`move`); introduce shared file-based lock store; add `show`/`search` CLI; rename `leaf` to `symbol` at the tool surface.
- **[T20260412-0645-2]** — Add compact `orbit.graph.overview` output for large repos.
- **[T20260412-0645-3]** — Architectural graph navigation: `deps`, `implementors`, `callers`.
- **[T20260416-0719]** — Recover from a corrupted default knowledge graph store.
- **[T20260417-0301-2]** — Harden graph lock/write/read tooling.
- **[T20260417-0307]** — Gate and guard graph refresh/search hot paths.
- **[T20260417-0639]** — Speed up workspace-init graph persistence hot path.
- **[T20260421-0358]** — Scope graph refs by branch.
- **[T20260421-0528]** — Historical `task_ids` schema on every node + git history walker for attribution; removed by [T20260506-11].
- **[T20260426-0139]** — Parallelize per-file hashing and leaf extraction while preserving deterministic graph output.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface and use task lock reservations as preflight write guards.
- **[T20260428-1]** — Historical graph task-ID attribution/search alignment; superseded by [T20260506-11].
- **[T20260506-11]** — Remove knowledge-graph task attribution; preserve task IDs as local commit-search keys.
- **[T20260430-22]** — Compact the knowledge-graph design docs and remove duplicate top-level narrative.
- **[T20260509-33]** — Skip symlinked directory entries during knowledge scanner traversal.
- **[T20260509-70]** — Build the SQLite secondary index sidecar during graph persistence.
- **[T20260509-72]** — Use the SQLite secondary index for current, unscoped `orbit.graph.overview` summary aggregation.
- **[T20260510-5]** — Move canonical knowledge-graph command semantics into `orbit_knowledge::commands::*`; keep `orbit-tools` as dispatch and envelope shaping.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
