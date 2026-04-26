# Knowledge Graph — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26 (read-only public graph surface, [T20260426-0453])

> *"Grep finds strings. An LSP finds symbols. A knowledge graph remembers what those symbols are, where they live, who touched them, and how they relate — in a form an agent can page into a 200k context window."*

The knowledge graph is Orbit's durable, queryable representation of a codebase. It sits between raw files on disk and the agent's prompt: a content-addressed, branch-scoped graph of directories, files, and extracted symbols, annotated with git history, lockable per-node, and overlaid with an ephemeral per-activity working copy during mutations.

The graph exists so agents can answer questions like *"where is `AgentRuntime` defined and what implements it?"* or *"which symbols did task [T20260421-0528] touch?"* without re-reading the repo from scratch and without guessing. It is deliberately narrower than an LSP and deliberately broader than a filename index.

This document is the entry point. [2_design.md](./2_design.md) specifies the current implementation in detail; [3_vision.md](./3_vision.md) captures open questions and direction.

---

## 1. Motivation

Unassisted agents navigate codebases with three tools: `grep`, `find`, and reading files in full. Each has a failure mode:

1. **Grep is string-level.** It cannot distinguish a call site from a comment, or a struct field from an unrelated variable of the same name. It scales in output volume, not precision.
2. **Full-file reads bloat context.** Reading `mod.rs` to answer "where is `run_build` defined" pulls 400 lines to find 30. Repeated across a session, this dominates cache pressure.
3. **Nothing remembers structure between sessions.** The agent re-derives the same module layout, the same trait hierarchy, the same import graph on every cold start. This is both slow and unstable — different sessions can reach different summaries of the same repo.

An LSP solves #1 and parts of #2 but is hostile to agent loops: it is stateful, process-local, and returns results tuned for IDE UX (hover text, rename previews) rather than for token-efficient prompt assembly.

The knowledge graph addresses all three by precomputing a structural index once per branch, persisting it in a content-addressed store, and exposing it through query tools whose outputs are shaped for prompt consumption.

---

## 2. Core Concepts

### 2.1 Node types

The graph is a tree with three node kinds:

- **`DirNode`** — a directory. Holds `dir_children` and `file_children`. The root dir's id derives from `"."`.
- **`FileNode`** — a source file. Holds `leaf_children`, `extension`, `language`, and a `source_blob_hash` pointing at the file's full text in the blob store.
- **`LeafNode`** — an extracted symbol (function, method, class, struct, interface, trait, impl, field, module). Holds its own `source`, `source_hash`, line range, input/output signature fields, and a history log.

All three share `BaseNodeFields`: `id`, `identity_key`, `location`, `language`, `description`, `parent_id`, lock state, and a sorted, deduplicated `task_ids` list attributed from commit history ([T20260421-0528]).

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
└── refs/heads/<branch>.json     mutable branch ref → active index
```

A rebuild writes new immutable objects/blobs and a fresh index, then atomically swings the branch ref. Refs are the only mutable surface, so concurrent builds on different worktrees cannot corrupt each other ([T20260421-0358]). See [specs/refs.md](./specs/refs.md) for the full resolution rules.

### 2.4 Working graph and write guards

The current public graph surface is read-only. Agents use it to inspect, search, and pack context, while write coordination happens before dispatch through task `context_files` and `orbit.task.locks.reserve` preflight guards. Those guards operate at the workspace/task plane rather than inside a branch-local graph ref, which keeps them meaningful when agents work in separate worktrees.

The in-memory **working graph** (`crates/orbit-knowledge/src/working_graph`) remains an internal/deferred mutation substrate. Keeping graph refs stable for the duration of an agent's reasoning window is still the desired property, but public graph write tools are not part of this version because branch-local graph locks do not coordinate independent worktrees.

### 2.5 Attribution

Each node carries a `task_ids` list. These are populated by the history-walker stage (`pipeline::history`, introduced in [T20260421-0528]) which parses `\[T\d{8}-\d{4}(?:-\d+)?\]` tags out of commit messages, maps hunks to symbols at the commit's tree, and accumulates the result onto the current graph. A merge commit where both sides touched the same symbol sets `structural_conflict: true` — informational only; git already resolved the textual conflict.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Crate boundary | `crates/orbit-knowledge` | [T20260411-0008], [T20260411-0424] |
| Storage layout | `src/graph/object_store.rs` | [T20260421-0358] |
| Build pipeline | `src/pipeline/` | [T20260411-0424], [T20260417-0639], [T20260426-0139] |
| History attribution | `src/pipeline/history.rs` | [T20260421-0528] |
| Query services | `src/service/` | [T20260412-0645-2], [T20260412-0645-3] |
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
- **[T20260421-0528]** — `task_ids` schema on every node + git history walker for attribution.
- **[T20260426-0139]** — Parallelize per-file hashing and leaf extraction while preserving deterministic graph output.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface and use task lock reservations as preflight write guards.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
