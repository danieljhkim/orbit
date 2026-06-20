---
summary: "Orbit Graph — Vision"
type: design
title: "Orbit Graph — Vision"
owner: claude
last_updated: 2026-06-13
status: Draft
feature: orbit-graph
doc_role: vision
tags: ["orbit-graph"]
related_features: [knowledge-graph]
---

# Orbit Graph — Vision

Forward-looking design space for `orbit-graph`. Everything below is hypothesis, not commitment. Items here do not carry task IDs because they are not yet scheduled; if an item lands, the task reference appears in [2_design.md](./2_design.md) when that doc is updated.

Current contracts and the V1 plan live in [2_design.md](./2_design.md) and [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md).

---

## 1. Open Questions

### 1.1 Write surface (V2)

The most consequential follow-on. V1 is read-only; agents edit files via normal tools and the graph reflects the result. A writeable graph would let agents perform structural edits at the graph level — rename across all resolved callers, replace a function body, move a symbol between files — and have the graph compile those edits into source patches.

A sketch of the V2 shape (pulled from the previous `GRAPH_DESIGN.md`, folded into this section on 2026-05-24):

```
   Source files
        │
        │  extract  (orbit-graph-extract)
        ▼
     Graph  (SQLite, derived)
        │
        │  overlay  (in-memory WorkingGraph)
        ▼
   Working Graph  (per-process, ephemeral)
        │
        │  compile  (patch compiler)
        ▼
   Source Patches
        │
        │  commit  (atomic temp-file + rename)
        ▼
   Source files (modified)
```

Five named transitions; each lives in one module. The V1 read model is deliberately compatible: per-worktree DB, qualified-name resolution, ephemeral symbol IDs all carry forward without a schema break.

**V2 edit set (proposed):**

| Operation | Payload | Compiles to |
|---|---|---|
| `Rename` | `target`, `new_name` | Replace identifier at definition + every resolved reference |
| `ReplaceBody` | `target`, `new_body` | Replace text between the symbol's body span |
| `Delete` | `target` | Remove symbol span; warn on remaining references |
| `InsertAfter` | `anchor`, `source` | Insert text after the anchor's end span |
| `Move` | `target`, `dest_file` | Delete from source, insert into dest, update imports |

**V2 concurrency model (proposed):** optimistic, file-grained, no locks. Two agents stage independent edits; on commit, each verifies the affected files' `content_hash` hasn't shifted since extraction. First commit wins; the second is rejected and the agent re-plans against fresh state. Working graphs are in-memory and never collide because they're per-process.

**Open V2 questions:**

- How are edits exposed to non-Rust callers? An MCP `orbit.graph.stage` / `orbit.graph.commit` pair is the obvious surface, but the payload shape for `Rename` vs `ReplaceBody` differs enough that one tool with a union type may be awkward.
- Should the working graph be persistable across processes? V1's answer is no — ephemeral, per session. But agents on long tasks may want to stage many edits and commit at the end of the session.
- Cross-language renames are out of V2's scope. A Rust symbol's TypeScript binding stays untouched unless V3 adds cross-language refs (§1.2).

### 1.2 Cross-language reference resolution

A Rust function called from TypeScript via N-API or napi-rs is two unrelated nodes in V1. Possible approaches: a pluggable "reference provider" trait with LSP as one backend, or a smaller specific-purpose binding-extractor that recognizes a handful of FFI patterns (napi-rs `#[napi]` macros, neon `cx.export_function`, pyo3 `#[pyfunction]`). The smaller targeted approach is more aligned with the structural-only ceiling V1 sets.

### 1.3 Embedding / semantic search overlay

`search` is FTS5 only in V1 — fast, deterministic, exact-string. A semantic overlay (vector embeddings of symbol docstrings + signatures) would help with intent-shaped queries like "where do we rate-limit requests?" Storage is the easy part; the harder questions are which embedding model is acceptable to ship, and how to express the union of FTS + semantic results without surprising agents who expect exact-match behaviour.

### 1.4 Cross-worktree dedup

If five worktrees on the same machine share unchanged files, V1 re-extracts five times. A shared content-addressed cache layer (keyed on `blake3(file_bytes) || extractor_version`) could deduplicate; the cost is reintroducing some of the complexity V1 deleted. Worth doing only if multi-worktree users complain.

### 1.5 Watcher reliability

`notify` has known issues on Linux with mass-rename operations (e.g. `git checkout` of a branch that moved 100 files). V1's hard freshness barrier is explicit `sync`; watcher errors schedule a conservative auto sync, and `SyncPolicy::Windowed` remains available only for callers that cannot run a watcher. We should still measure stale-result reports and event-miss patterns.

### 1.6 MCP daemon Graph handle lifetime

V1 expects the MCP server to keep a `Graph` handle open across calls (5ms per-call open cost is significant on a hot tool). Worth benchmarking: does the open handle accumulate cost over a long session (SQLite page cache, file descriptors)? Recycling on a TTL would be a safety valve if so.

### 1.7 Pack rendering separation

`KnowledgePack` in `orbit-knowledge` mixes query, budget, and prompt assembly. Lifting it into a separate `orbit-context` crate is the right shape — orbit-graph stays a query API, orbit-context owns budget-shaped output for prompt windows. Out of scope for the V1 migration, but the seam is preserved (the public Rust API in [2_design.md](./2_design.md) §8 returns structured types, not prompt-shaped strings).

---

## 2. Prior Work

### 2.1 Orbit-internal

- **`orbit-knowledge`** is the immediate predecessor and the system being replaced. Its design is documented at [docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/). The V1 spec ([`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md)§1) enumerates the specific failures driving the rewrite.
- **`orbit-search`** is the cross-cutting search surface that fronts task / doc / learning / ADR queries. `orbit graph search` should plug into the same retrieval shape so agents don't learn two query languages.

### 2.2 LSP-based code intelligence

Not pursued. LSP servers are stateful, process-local, and tuned for IDE UX (hover text, rename previews) rather than token-efficient prompt assembly. Reusing rust-analyzer or tsserver as a backend would bring in heavy dependencies, stateful daemons that complicate the per-worktree model, and result shapes designed for humans. The structural ceiling that tree-sitter imposes is the right trade for an agent-facing index — what we *don't* know is honestly modeled by the confidence ladder.

### 2.3 SCIP, tree-sitter, semantic graphs

[SCIP](https://github.com/sourcegraph/scip) (Sourcegraph's code intelligence index) is a precedent for "treat code intelligence as a derived, regenerable index." Orbit Graph differs by being per-worktree rather than cross-repo, by skipping precise type-aware analyses in favour of the confidence ladder, and by integrating with agent skill workflows rather than a code-search UI.

Tree-sitter itself is the parsing backbone in both V1 and V2.

### 2.4 git as the source of truth

The reframe in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §2 — "the graph is a derived index, not a source of truth" — borrows from git's own model: blobs are immutable, refs are cheap pointers, and any working tree can be reconstructed from the object database. Orbit Graph reuses the *shape* (one SQLite file = one snapshot, regenerable from disk) without reusing the *mechanism* (no objects, no refs, no merge model).

---

## 3. What May Be Distinctive

- **Per-worktree DB as the concurrency model.** Most code-intelligence systems are workspace-scoped and pay for locking or transactional coordination. Per-worktree state plus WAL is a structurally simpler answer that fits agent workflows (worktree-per-task is already the norm in Orbit).
- **Confidence as schema.** Most graphs treat ambiguity as a footnote — "results may be approximate." Encoding `exact` / `import_resolved` / `same_module` / `fuzzy_name` as a column with a documented ordering lets agents make filtering decisions explicit instead of hoping the underlying index made the right call.
- **Structural-only ceiling, honestly drawn.** Refusing trait dispatch / macro expansion / cross-language resolution in V1 (and saying so loudly in the contract) means the parts we *do* promise are reliable. Agents that need ground truth fall back to `rg`; the graph doesn't try to be a compiler.
- **Selector grammar as a stable public surface.** The selector parser lives in the extract crate, not the storage crate, so it survives storage redesigns. Skills can address symbols by selector across V1, V2, and beyond.

---

## 4. References

### 4.1 Orbit-internal

- [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) — the prescriptive V1 spec
- [docs/design/orbit-graph/2_design.md](./2_design.md) — design overview
- [docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/) — the predecessor (`orbit-knowledge`)
- [`ARCHITECTURE.md`](../../../ARCHITECTURE.md) — workspace crate layering rules

### 4.2 External

- [Sourcegraph SCIP](https://github.com/sourcegraph/scip) — precedent for derived code intelligence indexes
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/) — parsing backbone
- [SQLite WAL](https://www.sqlite.org/wal.html) — concurrency model

---

## Task References

- [ORB-00377] moved watcher-backed graph reads from open question to the V1 design contract; the remaining watcher concern is operational reliability measurement.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
