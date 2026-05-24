# Orbit Graph — Redesign Spec

**Status:** Draft proposal
**Relation to `orbit-knowledge`:** Coexists initially. Both crates run side-by-side under a feature flag; whether `orbit-knowledge` is eventually phased out depends on the head-to-head effectiveness measurement in §16 Step 4 — it is not a foregone conclusion of this spec.
**Author:** working from the V2 sketch in `GRAPH_V2.md` + the existing design in [`../../knowledge-graph/`](../../knowledge-graph/)
**Scope:** V1 — read-only graph. A writeable graph (Rename, ReplaceBody, Move, working-graph overlay, patch compiler) is V2, sketched in §17 and tracked in [`../3_vision.md`](../3_vision.md). The previous separate `GRAPH_DESIGN.md` describing the write surface has been folded into this spec on 2026-05-24 to remove the contradictory scope between the two docs.

---

## 1. Problem

The current `orbit-knowledge` crate (~24k LOC) is a cache pretending to be a versioned store. Concrete failures:

1. **Two storage paths must agree.** Content-addressed JSON objects under `objects/<hh>/<hash>.json` *and* a SQLite sidecar. They drift; agents see stale or contradictory results.
2. **Unshipped mutation layer.** ~1.5k LOC of `working_graph/` exists but isn't exposed publicly, because the lock protocol cannot coordinate independent worktrees.
3. **Locks that don't lock the right thing.** Same-branch worktrees still race (acknowledged in ADR-002's cost line).
4. **Full re-extraction on any file change.** No incremental refresh.
5. **Mixed concerns.** Query, mutation, durable storage, ref management, pack rendering, and task lineage all live in one crate.

Root cause: the graph was designed as a git-like history layer (durable refs, content-addressing, working trees) when the actual job is "fast, fresh, queryable index of the current code."

## 2. Reframe

> **The graph is a derived index, not a source of truth.**

Git is the source of truth. The graph is reproducible from `(commit_sha, dirty_file_set, extractor_version)` in seconds. That reframe deletes the need for atomic ref swaps, object dedup, durable working graphs, and custom lock protocols.

## 3. Goals

- **Deterministic.** Same input → same graph, byte-for-byte.
- **Cheap.** Cold build for orbit-sized repos under 3 seconds.
- **Fresh.** Incremental refresh under 50ms p95 per file change.
- **Concurrent-safe by construction.** Two worktrees on the same branch cannot corrupt each other.
- **Honest.** Edge confidence is part of the schema, not a footnote.
- **Small.** ≤7 query/sync commands on the agent-facing surface; admin commands (`version`, `db-path`, `clean`) are separate and not counted. Total crate footprint ≤10k LOC.

## 4. Non-goals

- Compiler-grade call resolution. Signature matching is the ceiling.
- Macro expansion (Rust proc-macros, TS decorators that generate code).
- Cross-language reference resolution (Rust ↔ TS via FFI: out of scope).
- Persistent history / time-travel queries. Use git for that.
- **Public mutation API in V1.** Agents edit files via normal tools; the graph reflects the result. A future writeable graph (Rename, ReplaceBody, Move, etc., with a working-graph overlay and patch compiler) is a V2 design — out of scope for this spec. See [`../3_vision.md`](../3_vision.md) for the V2 sketch.
- Embedding / semantic search. Separate concern.

## 5. Architecture

Three crates, replacing `orbit-knowledge`:

```
orbit-graph-extract    pure functions: (bytes, path) -> ExtractedFile
                       no I/O, no async, one module per language

orbit-graph            SQLite schema, build pipeline, query API
                       depends on -extract

orbit-graph-cli        CLI subcommands + MCP tool surface
                       depends on -graph
```

Layered the same way as the rest of the workspace per [`../../../../ARCHITECTURE.md`](../../../../ARCHITECTURE.md). No cross-crate edges beyond what's shown.

### 5.1 Mental model and crate boundaries

```
                 authoritative
              ┌────────────────┐
              │  Source files  │
              │  git-tracked   │
              └───────┬────────┘
                      │ read bytes during sync
                      ▼
          ┌────────────────────────┐
          │  orbit-graph-extract   │
          │  pure tree-sitter      │
          │  Extractor::extract()  │
          └───────┬────────────────┘
                  │ ExtractedFile
                  ▼
          ┌────────────────────────┐
          │      orbit-graph       │
          │  sync pipeline         │
          │  SQLite store          │
          │  query API             │
          └───────┬────────────────┘
                  │ JSON views
                  ▼
          ┌────────────────────────┐
          │   orbit-graph-cli      │
          │  orbit graph <cmd>     │
          │  MCP tool wrappers     │
          └────────────────────────┘
```

The only durable state owned by this feature is the SQLite graph DB. Source files are edited by normal agent/file tools, then `sync` re-derives the graph from disk. There is no working graph, no graph commit step, and no graph-owned mutation protocol in V1.

```
crates/
├── orbit-graph-extract/
│   └── src/
│       ├── lib.rs              # Extractor trait + registry
│       ├── extracted.rs        # ExtractedFile, RawSymbol, RawRef, ...
│       └── languages/          # rust, ts, python, go, java, ruby, c, csharp, kotlin, markdown, config
│
├── orbit-graph/
│   └── src/
│       ├── lib.rs              # Graph public API
│       ├── error.rs
│       ├── selector.rs         # selector parser kept for shared addressing
│       ├── store/              # schema, transactions, row reads/writes
│       ├── sync/               # scanner, diff, extraction pipeline, resolver
│       └── query/              # search, show, refs, callees, impact, trace
│
└── orbit-graph-cli/
    └── src/
        ├── main.rs
        ├── commands/           # sync, search, show, refs, callees, impact, trace
        └── mcp/                # 1:1 tool wrappers over command/query surfaces
```

## 6. Storage

### 6.1 Layout

```
.orbit/graph/
├── <branch>.<extractor_version>.db   # the only persistent artifact
└── <branch>.<extractor_version>.db-wal
```

One SQLite file per `(worktree, branch, extractor_version)`. No objects, no blobs, no refs directory, no JSON index files.

`<branch>` in the filename is sanitized — `/` is replaced by `_` so that `feat/foo` produces `feat_foo.42.db`, not a `feat/` subdirectory. The raw branch name is preserved in `meta.branch` for traceability.

**Worktree-scoped, not workspace-scoped.** Each git worktree gets its own DB. Disk cost is ~10MB per worktree for orbit-sized repos — negligible. Same-branch worktree contention is solved by not sharing state.

**Versioned by extractor.** When extractor logic changes, bump `EXTRACTOR_VERSION`. Old DBs become invisible and get deleted on next sync. No schema migrations to write.

### 6.2 Schema

```sql
-- Files actually indexed (post-orbitignore, post-language-filter).
CREATE TABLE files (
    path           TEXT PRIMARY KEY,
    content_hash   BLOB NOT NULL,      -- blake3 of file bytes
    mtime_ns       INTEGER NOT NULL,
    lang           TEXT NOT NULL,      -- "rust", "typescript", "python", ...
    byte_len       INTEGER NOT NULL,
    extracted_at   INTEGER NOT NULL
) STRICT;

-- Symbols defined in files.
CREATE TABLE symbols (
    id             INTEGER PRIMARY KEY,
    file_path      TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    name           TEXT NOT NULL,      -- "run_due_schedulers"
    qualified      TEXT NOT NULL,      -- "orbit_core::scheduler::run_due_schedulers"
    kind           TEXT NOT NULL,      -- "function" | "struct" | "enum" | "trait" |
                                       -- "impl" | "method" | "module" | "const" |
                                       -- "test" | "type_alias"
    span_start     INTEGER NOT NULL,   -- byte offset into file at content_hash
    span_end       INTEGER NOT NULL,   -- exclusive byte offset
    signature      TEXT,               -- one-line normalized signature
    parent_symbol  INTEGER REFERENCES symbols(id) ON DELETE CASCADE
) STRICT;
-- symbols.id is autoincrement and NOT stable across re-extracts.
-- Use `qualified` for stable cross-build identity. See §6.3.

CREATE INDEX symbols_name      ON symbols(name);
CREATE INDEX symbols_qualified ON symbols(qualified);
CREATE INDEX symbols_file      ON symbols(file_path);

-- Textual references from a source location to a symbol name.
-- Covers callers, type users, `use` statements, trait bounds — anything
-- anchored to (file, span). Resolution to a concrete symbol is by
-- `target_qualified` lookup, NOT by FK on symbols.id. `target_symbol_hint`
-- is a build-time cache that may go stale after incremental sync; queries
-- that need correctness re-resolve via `target_qualified`. See §6.3.
CREATE TABLE refs (
    id                  INTEGER PRIMARY KEY,
    from_file           TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    from_span_start     INTEGER NOT NULL,   -- byte offset
    from_span_end       INTEGER NOT NULL,   -- exclusive
    target_name         TEXT NOT NULL,      -- short name; fallback for fuzzy
    target_qualified    TEXT,               -- best-effort qualified name (authoritative)
    target_symbol_hint  INTEGER,            -- non-authoritative; no FK
    kind                TEXT NOT NULL,      -- "call" | "type" | "use" | "trait_bound"
    confidence          TEXT NOT NULL       -- see §11
) STRICT;

CREATE INDEX refs_target_qualified ON refs(target_qualified) WHERE target_qualified IS NOT NULL;
CREATE INDEX refs_target_name      ON refs(target_name);
CREATE INDEX refs_from_file        ON refs(from_file);

-- Symbol-to-symbol structural edges. No file:span source location.
-- Covers `impl Trait for Type`, class `extends`, interface `implements`,
-- supertype links. Both endpoints are qualified names; resolve to symbol
-- IDs at read time. Anchored to the file containing the relation's
-- definition site (e.g. the file with the `impl` block) for cascade.
CREATE TABLE relations (
    id              INTEGER PRIMARY KEY,
    from_qualified  TEXT NOT NULL,          -- concrete type / subclass
    to_qualified    TEXT NOT NULL,          -- trait / superclass / interface
    kind            TEXT NOT NULL,          -- "impl" | "extends" | "implements"
    def_file        TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    def_span_start  INTEGER NOT NULL,
    def_span_end    INTEGER NOT NULL,
    confidence      TEXT NOT NULL
) STRICT;

CREATE INDEX relations_from ON relations(from_qualified);
CREATE INDEX relations_to   ON relations(to_qualified);
CREATE INDEX relations_kind ON relations(kind);

-- Imports / use statements. Module-level dependency edges.
-- `target_path` is a language-specific opaque string. For Rust it's a
-- `::`-joined path ("orbit_core::scheduler"); for TS it's the import
-- specifier ("./utils/foo", "@orbit/core"); for Python it's the dotted
-- module path. Comparison is exact-string only; cross-language matching
-- is not in scope.
CREATE TABLE imports (
    from_file      TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    target_path    TEXT NOT NULL,
    target_symbol  TEXT                -- "Scheduler" or NULL for whole-module
) STRICT;

-- Clap / CLI command surface, extracted structurally.
CREATE TABLE commands (
    name           TEXT PRIMARY KEY,
    file_path      TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    span_start     INTEGER NOT NULL,
    handler_symbol INTEGER REFERENCES symbols(id)
) STRICT;

-- Notable string literals — error messages, log lines, route paths.
-- Filter: length >= 6, not all ASCII punctuation, not pure format string.
CREATE TABLE strings (
    id             INTEGER PRIMARY KEY,
    file_path      TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    line           INTEGER NOT NULL,
    value          TEXT NOT NULL,
    context_symbol INTEGER REFERENCES symbols(id)
) STRICT;

-- Config keys: YAML / TOML / JSON / env var references.
CREATE TABLE configs (
    id             INTEGER PRIMARY KEY,
    file_path      TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    line           INTEGER NOT NULL,
    key            TEXT NOT NULL,
    kind           TEXT NOT NULL       -- "yaml" | "toml" | "json" | "env" | "serde"
) STRICT;

-- Full-text search across the three high-value surfaces.
CREATE VIRTUAL TABLE symbols_fts USING fts5(name, qualified, signature, content='symbols');
CREATE VIRTUAL TABLE strings_fts USING fts5(value, content='strings');
CREATE VIRTUAL TABLE configs_fts USING fts5(key, content='configs');

-- Metadata.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
-- Stored keys:
--   "extractor_version"     extractor version this DB was built with
--   "schema_version"        schema version (independent of extractor)
--   "branch"                unsanitized git branch name
--   "commit_sha"            HEAD at last full build (best-effort; "" if detached / not a git repo)
--   "last_full_build_at"    epoch nanos
--   "last_incremental_at"   epoch nanos
```

### 6.3 Why this schema

- **Single source of truth.** Drop the file row → cascade deletes its symbols, refs, relations, strings, configs.
- **Resolution by qualified name, not by ID.** Symbol IDs are autoincrement and regenerate on every re-extract of their file. Cross-file references therefore look up symbols by `target_qualified` (or fall back to `target_name` for unresolved / fuzzy cases), never by FK on a symbol ID. `target_symbol_hint` is a build-time convenience for hot reads — queries that need correctness re-resolve by qualified name. This makes incremental sync's "delete and re-insert this file's symbol rows" safe: inbound refs in other files never see dangling IDs because they don't store IDs.
- **Two ref tables, not one.** `refs` is for `(file, span) → symbol` (calls, type uses, `use` statements, trait bounds). `relations` is for `symbol → symbol` structural edges (`impl Trait for Type`, `extends`, `implements`). Their shapes differ enough that merging them produces meaningless columns on the impl side. Keeping them split also makes "what implements X?" a `relations` lookup, which is a documented hot path.
- **No identity_key column.** The current graph carries one for cross-build lineage tracking. For a cache, qualified-name is sufficient identity; rename tracking is a separate feature on top of git.
- **Confidence is a string column, not an enum type.** SQLite has no enums; strings are fine and queryable. Canonical values + ordering live in §11.

## 7. Extraction

Per language, a pure function:

```rust
pub trait Extractor {
    fn lang(&self) -> &'static str;
    fn supports(&self, path: &Path) -> bool;
    fn extract(&self, path: &Path, bytes: &[u8]) -> ExtractedFile;
}

pub struct ExtractedFile {
    pub symbols:   Vec<RawSymbol>,
    pub refs:      Vec<RawRef>,       // (file, span) → name
    pub relations: Vec<RawRelation>,  // symbol → symbol (impl/extends/implements)
    pub imports:   Vec<RawImport>,
    pub strings:   Vec<RawString>,
    pub configs:   Vec<RawConfig>,
    pub commands:  Vec<RawCommand>,
}
```

Implementations live in `orbit-graph-extract::{rust, typescript, python, go, java, ruby, c, csharp, kotlin, markdown, config}`. Lift from the current `extract/` module — that work is already correct, just needs a different output shape.

Tree-sitter remains the backbone. No LSP, no rustc-as-a-library, no proc-macro expansion.

### 7.1 Cross-file reference resolution

Two-pass build:

1. **Pass 1:** Extract `ExtractedFile` per file in parallel. Insert files, symbols, and imports.
2. **Pass 2:** Walk raw refs. For each, compute `target_qualified` per the rules in §11 (canonical confidence ladder) and write the row. `target_symbol_hint` is filled when the qualified name maps to a unique symbol in this same build.

Resolution uses qualified-name lookups, not symbol-ID FKs (see §6.3). On incremental sync of file F, refs *from* F are recomputed; refs *to* F in other files are left alone — their `target_qualified` strings remain valid even when F's symbol IDs change. Stale `target_symbol_hint` values are tolerated; queries re-resolve through `target_qualified` whenever the hint misses. A full sync (`--full`) is the only path that re-warms every hint; doing so is cheap (§12) and not on the hot read path.

Relations are written during Pass 1 (they're inherent to the defining file). The resolver does not need to re-walk them.

## 8. Build / refresh model

### 8.1 Sync as the only build primitive

```bash
orbit graph sync [--full]
```

Idempotent. Compares mtime+content_hash for each indexable file:

- **Unchanged:** skip.
- **Modified:** re-extract, replace rows in one transaction.
- **New:** extract, insert.
- **Deleted:** drop the file row (cascade handles the rest).

`--full` ignores mtime and rehashes everything. Use when extractor version bumps or you suspect corruption.

The full-build path uses `rayon` for parallel extraction, single-writer transaction for inserts. Cold build target: **<3s for 200k LOC**.

### 8.2 Watcher (optional)

`orbit graph watch` runs `notify` with 200ms debounce, calls sync on changes. Useful for the MCP daemon and dev loops. **Not load-bearing** — every read path can call `sync` on demand cheaply.

### 8.3 Sync policy

Sync behaviour is a property of the `Graph` handle, chosen at `open` time and not negotiated inside the library:

```rust
pub enum SyncPolicy {
    /// Never auto-sync. Callers invoke `sync()` explicitly.
    /// Best for tests and one-shot CLI commands.
    Manual,
    /// Sync inline on every query. Simplest correctness story;
    /// pays the stat cost on every call.
    OnRead,
    /// Sync inline only if the last successful sync is older than `window`.
    /// Recommended for long-lived processes (MCP server, watch mode).
    Windowed { window: Duration },
}
```

Defaults by entry point:

| Entry point | Default policy |
|---|---|
| `orbit graph <cmd>` (CLI) | `Manual` — CLI users are explicit |
| MCP server | `Windowed { window: 500ms }` |
| `orbit graph watch` | `Manual` (watcher fires sync directly) |

The previous "10ms stat budget at 5000 files" heuristic is gone — it didn't scale and was an implicit contract baked into the library. Moving the policy out makes the decision explicit and testable.

### 8.4 Dirty files (uncommitted)

The graph reflects what's on disk, not what's in git. Uncommitted changes are indexed normally. There is no "staging" notion.

## 9. Query surface

Seven commands. Each maps 1:1 to an MCP tool.

```
orbit graph sync [--full]
orbit graph search <query> [--kind symbol|string|config] [--lang X]
orbit graph show <selector>
orbit graph refs <symbol> [--confidence exact|import|same_module|fuzzy]
                          [--kind call|type|use|trait_bound|impl|extends|implements]
orbit graph callees <symbol>
orbit graph impact <selector> [--depth N=3]
orbit graph trace <command-name> [--depth N=5]
```

### 9.1 `search`

FTS5 across symbols, strings, configs. Default returns top 20 by relevance. Output:

```json
{
  "matches": [
    {"kind": "symbol", "name": "run_due_schedulers",
     "path": "crates/orbit-core/src/scheduler/scheduler.rs", "line": 142},
    {"kind": "string", "value": "scheduler tick failed",
     "path": "crates/orbit-core/src/scheduler/runner.rs", "line": 88},
    ...
  ]
}
```

### 9.2 `show`

Selector grammar (kept from current crate, agents already know it):

```
symbol:<path>#<name>[:<kind>]
file:<path>
module:<qualified>
command:<name>
```

Returns source bytes + metadata. Bounded by a max-bytes budget.

### 9.3 `refs`

Returns all references to a symbol, grouped by confidence. Default filters out `fuzzy_name`. The command unions two underlying queries:

- **`refs` table** for callers, type users, `use`-statement targets, trait bounds (anchored to a `(file, span)`).
- **`relations` table** for impl/extends/implements edges (symbol→symbol, anchored to the defining file).

Output:

```json
{
  "target": {"name": "run_due_schedulers", "qualified": "...::run_due_schedulers"},
  "refs": [
    {"file": "...", "line": 88,  "kind": "call", "confidence": "exact"},
    {"file": "...", "line": 132, "kind": "call", "confidence": "import_resolved"}
  ],
  "relations": [
    {"from": "MockScheduler", "kind": "impl", "file": "...", "line": 14, "confidence": "exact"}
  ],
  "skipped_low_confidence": 3
}
```

Replaces today's `callers`, `implementors`, `deps`, `lineage` — they're all this one command with different filters. (`callers` → `refs --kind call`; `implementors` → `refs --kind impl`; `deps` → `refs --kind use`.)

### 9.4 `callees`

Outbound calls from a symbol. Walks `refs WHERE from_file = ? AND from_span_start >= symbol.span_start AND from_span_end <= symbol.span_end AND kind = 'call'`.

### 9.5 `impact`

BFS over the union of `refs` (inbound) and `callees` (outbound), plus `relations` for impl-driven edges. Default depth 3. Default confidence floor: `same_module` (matches `refs`' default — excludes only `fuzzy_name`). Returns a flat list of touched symbols ordered by graph distance, capped at 200.

### 9.6 `trace`

```bash
orbit graph trace job-run
```

Resolves command name to its handler symbol via `commands.handler_symbol`, then BFS over `callees` with depth 5. Returns the call tree as nested JSON.

Like `impact`, `trace` is capped at **200 visited nodes** regardless of `--depth`. When the cap fires, the response carries `truncated: true` and `visited_nodes: 200`; callers can split into multiple narrower traces (e.g. trace from a sub-handler). This keeps the response within reasonable context-window bounds — depth 5 with branching factor 5 is otherwise ~3k nodes worst-case.

This is the "structural feature expansion" capability — concrete, bounded, no semantic guessing.

## 10. Concurrency model

- **One writer, many readers.** SQLite WAL handles this natively.
- **Sync acquires a flock on the DB file.** If a sync is already running in this worktree, queue and coalesce. If a sync is running in *another worktree*, no contention — different DB file.
- **No graph-level lock module.** Deleted entirely.

## 11. Confidence and accuracy contract

**Canonical confidence ladder.** §6.2 (schema), §7.1 (resolver), and §9 (queries) all reference these names verbatim; this section is the only place they're defined.

| Rank | Confidence | Meaning | Default visible? |
|---|---|---|---|
| 1 | `exact` | Same file, unambiguous match on name + qualified path. | yes |
| 2 | `import_resolved` | Cross-file, reached through an explicit `use` / `import` statement on `from_file`. | yes |
| 3 | `same_module` | Cross-file within the same module path; name is unique without an import. | yes |
| 4 | `fuzzy_name` | Name matches but multiple candidates exist, or no import path can be established. | no (opt-in via `--confidence fuzzy`) |

Ordering is strict high→low. `import_resolved` outranks `same_module` deliberately: an explicit `use` is stronger evidence than module-namespace uniqueness, because wildcard re-exports can produce the same `same_module` candidate for unrelated symbols.

What we explicitly **don't** promise:

- Trait dispatch resolution. A call to `foo.method()` where `foo: impl Trait` records `kind=call, confidence=fuzzy_name`.
- Macro-generated symbols. `#[derive(Serialize)]` does not produce a `serialize` symbol.
- Reflective / dynamic dispatch in any language.

Agents that need stronger guarantees should fall back to `rg` and read source. This is documented in CLAUDE.md.

## 12. Performance budget (CI-enforced)

| Operation | Repo: orbit (~200k LOC) | Target |
|---|---|---|
| Cold full build | 200k LOC, 10 langs | < 3s |
| Incremental sync (no changes) | 5000 files stat | < 100ms |
| Incremental sync (1 file changed) | re-extract + write | < 50ms p95 |
| `search` | FTS5 over ~100k symbols | < 5ms p95 |
| `refs` | indexed lookup | < 10ms p95 |
| `impact` depth=3 | BFS over ~100 nodes | < 50ms p95 |
| Resident memory | sync + idle | < 100MB |
| DB size | 200k LOC, 10 langs | < 50MB |

**Measurement contract.**

- **Hardware.** Numbers above are for the CI runner profile (`ubuntu-24.04`, 4-core, 16GB). Local dev measurements are advisory and not gated.
- **Baseline source.** `bench/baselines.json` is checked into the repo. The regression gate compares a run against this committed baseline, **not** the previous merged run — otherwise the gate ratchets up to whatever the last commit happened to measure and the budget silently erodes.
- **Updating the baseline** requires a PR with the `bench-baseline-bump` label and a one-line justification in the PR body. Routine perf wins → bump down; routine drift → no bump, fix the regression instead.
- **Wire `graph_bench.rs`** (already exists) to CI; results are written to `target/bench/` artifacts and diffed against `bench/baselines.json`. Gate fires when any row is >20% slower than baseline.

## 13. Public Rust API

```rust
// orbit-graph crate root

pub struct Graph { /* opaque */ }

impl Graph {
    pub fn open(worktree_root: &Path, policy: SyncPolicy) -> Result<Self, GraphError>;
    pub fn sync(&self, mode: SyncMode) -> Result<SyncReport, GraphError>;

    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Match>, GraphError>;
    pub fn show(&self, sel: &Selector) -> Result<Option<NodeView>, GraphError>;
    pub fn refs(&self, sel: &Selector, opts: &RefOpts) -> Result<RefResult, GraphError>;
    pub fn callees(&self, sel: &Selector) -> Result<Vec<CalleeEdge>, GraphError>;
    pub fn impact(&self, sel: &Selector, depth: u8) -> Result<ImpactResult, GraphError>;
    pub fn trace(&self, command: &str, depth: u8) -> Result<TraceResult, GraphError>;
}

pub enum SyncMode { Auto, Full }
pub enum SyncPolicy { Manual, OnRead, Windowed { window: Duration } }

pub struct SyncReport {
    pub files_indexed: usize,
    pub files_changed: usize,
    pub files_removed: usize,
    pub duration: Duration,
}
```

No async on the public surface. SQLite + tree-sitter are both sync. If the MCP server needs async, it wraps with `spawn_blocking`.

## 14. What we keep from the current crate

- All tree-sitter extractors (`extract/*.rs`). Move them into `orbit-graph-extract`, adjust output to `ExtractedFile`.
- `Selector` grammar and parser. Agents and skills know the syntax. **Every form currently used in `~/.claude/` skills and `.claude/skills/` must continue to parse identically** — this is a hard contract, not a best-effort. A pre-Step-1 audit captures the full grammar surface in `crates/orbit-graph-extract/src/selector.rs` as the canonical reference; any divergence is a release blocker for Step 3.
- The `graph_bench.rs` harness.
- `.orbitignore` defaults from `lib.rs`.
- Signature-matching approach for cross-file refs.

## 15. What we delete

| Module | LOC | Why |
|---|---|---|
| `graph/object_store.rs` | ~1000 | Content-addressed JSON, replaced by SQLite rows |
| `working_graph/` | ~1500 | Not publicly shipped; agents edit files instead |
| `lock.rs` | ~400 | Per-worktree DB + WAL replaces it |
| `pipeline/persist.rs` + half of `build.rs` | ~700 | File-level transactions replace bespoke pipeline |
| `service/lineage.rs` | ~250 | Task attribution already removed |
| `commands/write.rs`, `workflows/` | ~? | No mutation surface |
| Most of `store.rs` (pack rendering) | ~? | Move to context layer (separate concern) |

Estimated landing: ~24k → ~10k LOC. More capability (string / command / config indexes), fewer surfaces.

## 16. Migration plan

A four-step Orbit epic. Each step is one or more tasks; each task is independently shippable.

**Step 1 — Lift extractors (no behavior change).**
Create `orbit-graph-extract`. Move language modules from `orbit-knowledge::extract`. Adjust output shape to `ExtractedFile`. Keep `orbit-knowledge` as the only consumer for now.

**Step 2 — Land `orbit-graph` behind a feature flag.**
New crate, full schema, full query surface. MCP tools accept `ORBIT_GRAPH_BACKEND=v2` env var to switch. Old crate remains default. Dual-run for one release cycle; compare outputs in CI.

**Equivalence relation.** A `tools/graph-equiv` binary runs both backends against a frozen corpus of ~30 representative selectors covering rust, ts, python, and go, and fails CI on any diff outside the documented tolerances:

| Query | v1 vs v2 must agree on |
|---|---|
| `search <q>` | result set as unordered set of `(kind, file, name)` triples. v2 may surface additional match kinds (string, config) — extras are ignored, missing v1 matches fail the check. |
| `show <sel>` | source bytes byte-equal |
| `refs <sym>` | set of `(file, line, kind)` triples filtered to `confidence >= same_module`. v2 may surface *fewer* fuzzy matches than v1; differences below the confidence floor do not fail. |
| `callees <sym>` | set of `(file, line, target_name)` triples |
| `impact <sym>` (depth=3) | set of touched symbol qualified names |

Promotion to default (Step 3) requires zero diffs for a full release cycle. Per-query waivers — if any prove necessary — are documented in `bench/equiv-waivers.md` with rationale, and the waiver itself blocks until reviewed.

**Step 3 — Flip the default to v2.**
After equivalence holds for a week of real agent usage, flip the default backend to v2. `orbit-knowledge` remains reachable via env var indefinitely — this step opens the head-to-head evaluation window for Step 4, it does not commit to deletion.

**Step 4 — Measure effectiveness, then decide.**
Run a head-to-head measurement harness (`tools/graph-effectiveness/`, separate from the equivalence harness in `tools/graph-equiv/`) over a defined evaluation window of at least one full release cycle. Signals that matter:

| Signal | Operationalization |
|---|---|
| Task-completion success rate | Agent runs over a fixed task corpus; pass/fail rate per backend |
| Query latency by kind | Median + p95 for `search`, `refs`, `callees`, `impact`, `trace` |
| Coverage gaps | Queries that succeed against one backend and miss against the other — catalogued and triaged |
| Token cost per resolved query | Response size + payload shape across the same corpus |
| Operational quality | Manual reindex frequency, stale-result reports, lock contention incidents |

The output is a **decision artifact**, not a deletion: keep both crates, deprecate `orbit-knowledge` gradually, or remove it. **Phase-out is downstream of measurement, not a foregone conclusion.** No deletion task is allocated up front; the measurement results determine whether that work happens at all, and on what timeline.

Estimated calendar time: 6–8 weeks for Steps 1–3 driven by a single agent. Step 4's calendar is the length of the evaluation window plus the decision turnaround. The bulk of *technical* risk lives in Step 2; the bulk of *organizational* commitment lives in Step 4.

## 17. Open questions

These are deliberately deferred — not blockers for shipping the spec, but listed so they don't get lost.

1. **MCP daemon model.** Does the MCP server keep a `Graph` handle open across calls, or open-on-each-call? Open-on-each is simpler but adds ~5ms per call. Likely answer: keep open, sync on request, but worth benchmarking.
2. **Pack rendering.** Today `KnowledgePack` mixes query, budget, and prompt assembly. Lifting it into a separate `orbit-context` crate is right but out of scope for this spec.
3. **Cross-worktree dedup.** If 5 worktrees on the same machine share unchanged files, we re-extract 5 times. Acceptable today; revisit if a user complains.
4. **Watcher reliability.** `notify` has known issues on Linux with mass-rename operations. `SyncPolicy::Windowed` is the safety net; we should still measure.
5. **V2 write surface.** Rename, ReplaceBody, Delete, InsertAfter, Move — with an in-memory working-graph overlay, optimistic per-file hash verification on commit, and a patch compiler that turns graph edits into source diffs. Sketch lives in [`../3_vision.md`](../3_vision.md) §1.1. Out of scope for V1, but the V1 read model (per-worktree DB, qualified-name resolution, ephemeral symbol IDs) is deliberately compatible with adding writes later without a schema break.

## 18. What this spec deliberately does *not* include

- A philosophy section. The principles ("structural not semantic," "deterministic," etc.) are inherited from `GRAPH_V2.md` and the existing knowledge-graph ADRs. They don't need restating.
- A long list of "why not LSP." Not actually a live option in this codebase.
- Embedding / semantic search plans. Separate spec if and when needed.
- A rollback story beyond Step 3's env var. If we get to Step 4 and need to roll back, that's a crisis, not a planned mode.

