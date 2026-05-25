---
summary: "Orbit Graph — Design"
type: design
title: "Orbit Graph — Design"
owner: claude
last_updated: 2026-05-24
status: Draft
feature: orbit-graph
doc_role: design
tags: ["orbit-graph"]
related_features: [knowledge-graph]
---

# Orbit Graph — Design

This document specifies the design of `orbit-graph` at the architectural level: crate layout, storage layout, extraction model, sync semantics, query surface, and concurrency model. It is paired with the prescriptive working spec at [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md), which carries the full schema, performance budget, migration plan, and equivalence rules. When the two diverge, the working spec wins until this design doc is updated to absorb the change.

The companion ADR log is in [4_decisions.md](./4_decisions.md); forward-looking work (write surface, embeddings, cross-language refs) lives in [3_vision.md](./3_vision.md).

---

## 1. Crate Boundaries

Three crates, replacing `orbit-knowledge`. Layered per [`ARCHITECTURE.md`](../../../ARCHITECTURE.md); no cross-crate edges beyond what's shown.

```
                 authoritative
              ┌────────────────┐
              │  Source files  │
              │  git-tracked   │
              └───────┬────────┘
                      │ read bytes during sync
                      ▼
          ┌────────────────────────┐
          │  orbit-graph-extract   │   pure tree-sitter
          │  Extractor trait       │   no I/O, no async
          └───────┬────────────────┘
                  │ ExtractedFile
                  ▼
          ┌────────────────────────┐
          │      orbit-graph       │   SQLite store + query API
          │  Graph::open/sync/...  │   build pipeline
          └───────┬────────────────┘
                  │ JSON views
                  ▼
          ┌────────────────────────┐
          │   orbit-graph-cli      │   subcommands + MCP wrappers
          └────────────────────────┘
```

| Crate | Owns | Doesn't own |
|---|---|---|
| `orbit-graph-extract` | Tree-sitter parsing, language-specific extractors, the `ExtractedFile` shape, the `Selector` parser | Storage, queries, I/O, async |
| `orbit-graph` | Schema, transactions, sync pipeline, query API, confidence resolution | CLI, MCP, language-specific parsing |
| `orbit-graph-cli` | Argument parsing, JSON formatting, MCP tool registration | Anything semantic about the graph |

The selector parser lives in the extract crate (not the graph crate) because skills and downstream callers parse selectors before any DB exists, and the selector grammar is part of the public contract independent of storage.

## 2. Storage

The only durable artifact is the SQLite database.

```
.orbit/graph/
├── <branch>.<extractor_version>.db   # the only persistent artifact
└── <branch>.<extractor_version>.db-wal
```

One file per `(worktree, branch, extractor_version)`. `<branch>` in the filename is sanitized — `/` → `_` — so that `feat/foo` produces `feat_foo.42.db` instead of creating a `feat/` subdirectory. The unsanitized branch name is preserved in `meta.branch`.

**Worktree-scoped, not workspace-scoped.** Each git worktree gets its own DB. Same-branch worktree contention is solved by not sharing state; disk cost is ~10MB per worktree for orbit-sized repos.

**Versioned by extractor, not migrated.** When extractor logic changes, bump `EXTRACTOR_VERSION`. Old DBs become invisible and are deleted on next sync. No schema migrations to write or test.

The full schema — `files`, `symbols`, `refs`, `relations`, `imports`, `commands`, `strings`, `configs`, plus FTS5 virtual tables and a `meta` keystore — lives in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §6.2. Two design choices that earn their own discussion here:

### 2.1 Symbol IDs are ephemeral; resolution is by qualified name

Symbol primary keys are `INTEGER PRIMARY KEY` autoincrement. They regenerate every time a file is re-extracted. Inbound foreign keys on `symbols.id` are therefore deliberately avoided: `refs` and `relations` resolve targets by *qualified name*, not by ID. A `target_symbol_hint INTEGER` column exists on `refs` as a build-time cache, but it is non-authoritative — queries must re-resolve through `target_qualified` whenever the hint misses or doesn't match.

This makes incremental sync's "delete and re-insert this file's symbol rows" safe: refs in other files never see dangling IDs because they don't store IDs.

The cost: every cross-file lookup is a string index probe instead of a join. SQLite's FTS5 / B-tree indexes make this fast enough; see the performance budget in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §12.

### 2.2 Two ref tables, not one

`refs` is for **(file, span) → name** edges — calls, type uses, `use` statements, trait bounds. The source location is meaningful and queryable.

`relations` is for **symbol → symbol** edges — `impl Trait for Type`, class `extends`, interface `implements`. There is no useful `from_span` because the relation is between symbols, not between a source location and a symbol. Anchoring to the file containing the relation's definition site (e.g. the `impl` block's file) gives cascade-on-delete semantics without pretending the relation is anchored to a span.

Forcing impl edges into the `refs` table — as the original draft did — produces meaningless `from_span` columns and obscures the genuinely different lookup patterns ("who calls X?" vs "what implements X?"). The split also makes "what implements X?" a single `relations` index lookup, which is a documented hot path for refactor planning.

## 3. Extraction

Per language, a pure function:

```rust
pub trait Extractor {
    fn lang(&self) -> &'static str;
    fn supports(&self, path: &Path) -> bool;
    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile;
}

pub struct ExtractedFile {
    pub symbols:   Vec<RawSymbol>,
    pub refs:      Vec<RawRef>,
    pub relations: Vec<RawRelation>,
    pub imports:   Vec<RawImport>,
    pub strings:   Vec<RawString>,
    pub configs:   Vec<RawConfig>,
    pub commands:  Vec<RawCommand>,
}
```

Implementations live in `orbit-graph-extract::languages::{rust, typescript, python, go, java, ruby, kotlin, c, csharp, markdown, config}`. The existing tree-sitter extractors in `orbit-knowledge::extract` are correct; lifting them is largely a refactor that reshapes the output to `ExtractedFile`.

No LSP, no rustc-as-a-library, no proc-macro expansion. This is a hard structural ceiling — what we *don't* know is honestly modeled by the confidence ladder rather than papered over.

### 3.1 Two-pass build

1. **Pass 1 (parallel).** Extract `ExtractedFile` per file using `rayon`. Insert files, symbols, imports, relations, strings, configs, commands into the DB.
2. **Pass 2.** Walk raw refs and resolve `target_qualified` per the confidence rules (§4). Fill `target_symbol_hint` when the qualified name maps to a unique symbol in this same build.

On incremental sync, only files whose `content_hash` changed are re-extracted. Refs *from* a re-extracted file are rewritten; refs *to* that file in other files are left alone — their `target_qualified` strings remain valid even when symbol IDs change. Stale `target_symbol_hint` values are tolerated and re-resolved at query time.

A full sync (`--full`) is the only path that re-warms every hint. It's cheap (<3s for 200k LOC per the budget) and reserved for the explicit case.

## 4. Confidence Ladder

The canonical definition lives in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §11. Summary:

| Rank | Confidence | Meaning |
|---|---|---|
| 1 | `exact` | Same file, unambiguous match on name + qualified path |
| 2 | `import_resolved` | Cross-file, reached through an explicit `use` / `import` on `from_file` |
| 3 | `same_module` | Cross-file within the same module path; name unique without an import |
| 4 | `fuzzy_name` | Name matches but multiple candidates exist, or no import path |

Default query floor is `same_module` (excludes only `fuzzy_name`). Agents opt into fuzzy results explicitly. `import_resolved` outranks `same_module` because an explicit `use` is stronger evidence than module-namespace uniqueness, especially in the presence of wildcard re-exports.

What we explicitly *don't* promise: trait dispatch (`foo.method()` where `foo: impl Trait` is `fuzzy_name`), macro-generated symbols (`#[derive(Serialize)]` does not produce a `serialize` entry), and reflective / dynamic dispatch in any language. Agents needing stronger guarantees fall back to `rg` and source reads; this is documented in `CLAUDE.md`.

## 5. Sync Model

```bash
orbit graph sync [--full]
```

Idempotent. Compares mtime + content_hash per file:

- **Unchanged:** skip.
- **Modified:** re-extract, replace rows in a single transaction.
- **New:** extract, insert.
- **Deleted:** drop the file row (cascade handles symbols, refs, relations, strings, configs).

`--full` ignores mtime and rehashes everything. Cold-build target: <3s for 200k LOC.

### 5.1 Sync policy

Sync behaviour is a property of the `Graph` handle, set at `open` time:

```rust
pub enum SyncPolicy {
    Manual,                              // never auto-sync
    OnRead,                              // sync inline on every query
    Windowed { window: Duration },       // sync if older than window
}
```

CLI default: `Manual` (CLI users are explicit). MCP server default: `Windowed { window: 500ms }`. `orbit graph watch` uses `Manual` because the watcher fires sync directly.

The previous hardcoded "10ms stat budget, 500ms cache window" heuristic was an implicit contract baked into the library that didn't scale past ~5000 files. Moving the decision to `open` makes it explicit, testable, and per-entry-point.

## 6. Query Surface

Seven commands, each mapped 1:1 to an MCP tool. Specified in detail in [`GRAPH_SPEC.md`](./specs/GRAPH_SPEC.md) §9.

```
orbit graph sync     [--full]
orbit graph search   <query>   [--kind symbol|string|config] [--lang X]
orbit graph show     <selector>
orbit graph refs     <symbol>  [--confidence ...] [--kind ...]
orbit graph callees  <symbol>
orbit graph impact   <selector> [--depth N=3]
orbit graph trace    <command> [--depth N=5]
```

Bounded outputs: `impact` and `trace` both cap at 200 visited nodes regardless of `--depth`. When the cap fires, the response carries `truncated: true` so callers can split into narrower queries.

`refs` unions queries against the `refs` table (calls/type/use/trait_bound) and the `relations` table (impl/extends/implements). CLI `--kind impl` is a routing alias to `relations`. This means the seven-command surface absorbs what `orbit-knowledge` exposed as separate `callers`, `implementors`, `deps`, and `lineage` queries.

The `Selector` grammar is preserved verbatim from `orbit-knowledge` — every form used in existing skills (`symbol:<file>#<name>:<kind>`, `file:<path>`, `module:<qualified>`, `command:<name>`) must continue to parse identically. A pre-Step-1 audit of `.claude/skills/` captures the full grammar surface as the canonical reference.

## 7. Concurrency

- **One writer, many readers.** SQLite WAL handles this natively.
- **Sync acquires a flock on the DB file.** If a sync is already running in this worktree, the second caller queues and coalesces. If a sync is running in *another worktree*, no contention — different DB file.
- **No graph-level lock module.** Deleted entirely; the per-worktree DB + WAL replaces the previous `lock.rs` (~400 LOC).

## 8. Public Rust API

```rust
pub struct Graph { /* opaque */ }

impl Graph {
    pub fn open(worktree_root: &Path, policy: SyncPolicy) -> Result<Self, GraphError>;
    pub fn sync(&self, mode: SyncMode) -> Result<SyncReport, GraphError>;

    pub fn search(&self, q: &SearchQuery)            -> Result<SearchResult, GraphError>;
    pub fn show(&self, sel: &Selector, max_bytes: usize) -> Result<Option<NodeView>, GraphError>;
    pub fn refs(&self, sel: &Selector, opts: &RefOpts)-> Result<RefResult, GraphError>;
    pub fn callees(&self, sel: &Selector)             -> Result<Vec<CalleeEdge>, GraphError>;
    pub fn impact(&self, sel: &Selector, depth: u8)   -> Result<ImpactResult, GraphError>;
    pub fn trace(&self, command: &str, depth: u8)     -> Result<TraceResult, GraphError>;
}
```

No async on the public surface. SQLite and tree-sitter are both sync. If the MCP server needs async, it wraps with `spawn_blocking`.

## 9. Concerns & Honest Limitations

- **No compiler-grade call resolution.** Trait dispatch in Rust, virtual method dispatch in Java/C#, duck typing in Python — all degrade to `fuzzy_name`. The confidence ladder is the contract; agents that need ground truth use `rg` and read source.
- **No macro expansion.** Rust proc-macros and TypeScript decorators that generate code are invisible to the extractor. Symbols emitted by `#[derive(Serialize)]` are not in the graph.
- **No cross-language refs.** A Rust function called from TypeScript via FFI/N-API is two unrelated nodes in the graph. Cross-language matching is a separate problem (see [3_vision.md](./3_vision.md) §1.2).
- **No persistent history.** Use git for time travel. The graph reflects the *current* on-disk state of the worktree.
- **Same-machine worktree dedup is not done.** Five worktrees on the same machine re-extract unchanged files five times. Acceptable today; revisit if it bites.
- **Watcher reliability.** `notify` has known issues on Linux with mass-rename operations. `SyncPolicy::Windowed` is the safety net; we should still measure.
- **No public mutation API in V1.** Agents edit files via normal tools; the graph reflects the result. The V2 write surface — Rename, ReplaceBody, Move, with a working-graph overlay and patch compiler — is sketched in [3_vision.md](./3_vision.md) §1.1 but deliberately not in the V1 contract.

---

## Task References

No Orbit tasks have been allocated for this feature yet.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
