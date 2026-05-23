---
summary: "Knowledge Graph — Design"
type: design
title: "Knowledge Graph — Design"
owner: claude
last_updated: 2026-05-17
status: Draft
feature: knowledge-graph
doc_role: design
tags: ["knowledge-graph"]
---

# Knowledge Graph — Design

This document specifies the current knowledge graph: storage, build pipeline, query services, Orbit integration, locking, and limitations. The [T20260430-22] cleanup removes duplicated rationale already covered by [1_overview.md](./1_overview.md), [3_vision.md](./3_vision.md), and ADRs.

---

## 1. On-Disk Layout

```
.orbit/knowledge/
├── manifest.json                checkout identity + generated_at timestamp
├── refresh.lock                 flock for single-flighted rebuilds
├── refresh_state.json           debounce bookkeeping for dirty refreshes
├── graph/
│   ├── objects/<hh>/<hash>.json   immutable node bodies (dir / file / leaf)
│   ├── blobs/<hh>/<hash>.txt      immutable source blobs
│   ├── index/by-id/<root-graph-hash>.json   immutable per-build index
│   ├── graph_index.sqlite         mutable SQLite secondary index for fast reads
│   └── refs/heads/<branch>.json   mutable branch ref → active index
└── working/<activity_id>/       (in memory today; see 3_vision.md §2.3)
```

The split between immutable objects/blobs/index and mutable refs is deliberate. Rebuilds always write new immutable artifacts and then atomically rename a ref file; no object file is ever overwritten. This survives concurrent worktree rebuilds and interrupted writes ([T20260421-0358]).

`graph/graph_index.sqlite` is a mutable secondary index written alongside the content-addressed graph ([T20260509-70], [T20260509-72], [T20260509-73]). Its `meta` table records `schema_version`, `created_at`, and the active `graph_ref` root hash; `node` contains one row per dir/file/leaf with lowercased name/location fields, language, stable selectors where unambiguous, sibling ordinal, and graph scan order; `file_summary` stores one row per file with direct leaf-child count. Writes rebuild the SQLite schema/data in one WAL-backed transaction and insert `meta.graph_ref` last, so readers can treat a missing or mismatched `graph_ref` as invalid. Current read use is intentionally narrow: selector context, broad `overview` summaries, and conservative exact-name/path-prefix `orbit.graph.search` query shapes can use the sidecar when it matches the resolved graph ref; scoped overview summaries, substring search, and source-regex behavior still fall back to the JSON/object path.

The legacy `refs/current.json` layout is migrated to `refs/heads/<default-branch>.json` on open; see [specs/refs.md](./specs/refs.md).

---

## 2. Build Pipeline

The pipeline is a straight-line sequence of functions over a shared `PipelineContext`:

```
scan → hash → detect_changes → build_dirs → build_files → build_leaves → persist → write_manifest → save_hash_cache
```

| Stage | Output | Notes |
|-------|--------|-------|
| `scan_repo` | `ctx.file_paths` | Walks the repo honoring `.gitignore` plus Orbit-specific `.orbitignore` rules; classifies entries with `DirEntry::file_type()` and skips symlinked directories/files instead of following them ([T20260509-33]) |
| `compute_hashes` | Per-file content hashes | Drives incremental detection; reads and hashes files in parallel, then publishes `ctx.new_hashes` after the worker phase ([T20260426-0139]) |
| `detect_changes` | Added / modified / unchanged path set | Incremental leaf extraction uses this set to decide what can reuse the prior graph |
| `build_graph_dirs` | `DirNode` entries with parent/child wiring | Deterministic; root dir id derived from `.` |
| `build_graph_files` | `FileNode` entries linked to parent dir | Language detected from extension |
| `build_graph_leaves` | `LeafNode` entries via file-kind-dispatched extractor | Code via tree-sitter (C, C#, Rust, Python, Go, Java, JavaScript, Kotlin, TypeScript, TSX, Ruby); markdown ATX headings via the doc extractor added in [T20260422-1540]. YAML/JSON/TOML/CSV/TSV files are still classified via `FileKind::Config(_)` / `FileKind::Table(_)` but their extractors return zero leaves — the file node carries source for substring search and `show` ([T20260509-64]). Code coverage extensions: TypeScript/TSX [T20260505-11]; C# [T20260505-13]; C [T20260505-16]; Kotlin [T20260505-14]; Ruby [T20260505-15]. Per-file read/extract work runs in parallel, then graph mutation is merged on the main thread in file order ([T20260426-0139]). Incremental builds reuse unchanged file/leaf snapshots from the same branch ref when hashes match ([T20260426-0140]). Every extractor finalizes per-file unique leaf IDs through the [specs/leaf-id-uniqueness.md](./specs/leaf-id-uniqueness.md) scheme ([T20260510-7]) |
| `persist_graph` | Content-addressed objects, blobs, JSON index, SQLite secondary index | Atomic file writes for immutable JSON artifacts; SQLite sidecar rebuilt in one WAL transaction with `meta.graph_ref` written last ([T20260509-70]) |
| `write_manifest` | `manifest.json` | Timestamp + clean checkout identity + graph summary |
| `save_hash_cache` | `hashes.json` | Baseline for the next incremental `detect_changes` pass |

Extraction dispatches on `FileKind`. Each `FileExtractor` emits `ExtractedLeaf` records with name, kind, span, hash, and child names. Code uses tree-sitter; the markdown extractor emits ATX heading sections only ([T20260422-1540]). Config (YAML/JSON/TOML) and table (CSV/TSV) extractors classify the file but emit zero leaves; the file node still carries source so substring search and `show` resolve to the file ([T20260509-64]). Leaf IDs must be unique per file after extraction finalization: no two leaves for the same path may share `(qualified_name, kind)`, because the node ID is derived from `symbol:{path}#{qualified_name}:{kind}`. The contract and selector-breaking implications are specified in [specs/leaf-id-uniqueness.md](./specs/leaf-id-uniqueness.md) ([T20260510-7]).

Hashing and leaf extraction are parallelized only across independent file work. `compute_hashes` collects `(path, sha256)` worker results before replacing `ctx.new_hashes`; `build_graph_leaves` workers return either a reusable prior snapshot or freshly extracted file output, and the main thread applies those outputs sorted by original `FileNode` index. That merge discipline keeps `ctx.graph.leaves` and each `FileNode.leaf_children` byte-stable relative to the old sequential order while avoiding locks around `PipelineContext` ([T20260426-0139]). Individual file read failures remain non-fatal skips in both phases.

### 2.1 Incremental refresh

`ensure_fresh(knowledge_dir, repo_path)` is the read-side entry point. For clean worktrees, it compares the current git checkout identity (`HEAD` commit OID, with tree OID persisted for diagnostics/fallback) against the current branch ref. Commit timestamps remain diagnostic metadata and are not freshness authority. Dirty worktrees still use the dirty-worktree fingerprint, and the planner picks one of:

- **Fresh** — nothing to do.
- **SkippedDirtyDebounce** — worktree is dirty with the same fingerprint as a recent refresh; reuse.
- **Rebuild** — incremental if a manifest exists, full otherwise.
- **SkippedConcurrent** — another process already holds the refresh lock; wait briefly and reuse its result.

The refresh lock is a `flock` on `.orbit/knowledge/refresh.lock`. Concurrent callers wait for the in-flight build or reuse the pre-existing graph once the lock-holder publishes. The debounce window is `ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS` (default 5s).

Incremental rebuilds still rebuild the directory and file node skeleton from the current scan so deletes and `.orbitignore` changes take effect. The expensive leaf phase loads the previously persisted graph for the same branch ref and copies unchanged files' `source_blob_hash`, hydrated source, exports, re-exports, `leaf_children`, and leaf nodes when both the file source hash and every reused leaf's `file_hash_at_capture` match the new hash. Paths in `ctx.changed_paths`, new files, hash mismatches, absent refs, or unreadable prior graphs take the full extractor path instead ([T20260426-0140]).

Refresh hardening: [T20260417-0307] gated and guarded the refresh/search hot paths; [T20260416-0719] added recovery from a corrupted default store; [T20260417-0639] sped up the workspace-init persistence path; [T20260509-34] moved clean-checkout freshness from commit timestamps to exact git identity.

### 2.2 Removed task attribution ([T20260506-11])

The graph no longer stores node-level task attribution. The former `attribute_history` stage, `TaskIdPattern`, `task_ids`, `structural_conflict`, sidecar task-commit index, and `last_attributed_commit` cursor were removed after a 10-day audit found 0 uses of the reverse-lookup parameters across 961 `orbit.graph.*` tool calls. Existing graph objects or refs that still contain those legacy fields load through serde's unknown-field tolerance, and the fields disappear on rebuild.

`knowledge.task_id_pattern` is now a deprecated config key. Existing configs that still set it load successfully and emit a one-line warning to stderr; the value is ignored even when empty or no longer a valid regex.

Forward lookup remains a git convention: commits associated with a local Orbit task include `[T<task-id>]`, and operators can run `git log --grep '[T...]'` in their own workspace. Cross-engineer references use task `external_refs`.

### 2.4 Inclusion vs Access: `.orbitignore` vs Policy

The scan stage now evaluates two inclusion layers before a file ever reaches the parser:

1. Git's own ignore rules via `git check-ignore --stdin`.
2. Orbit's scan-only `.orbitignore` matcher inside `orbit-knowledge`, implemented with the `ignore` crate and composed from built-in defaults plus any root or nested `.orbitignore` files ([T20260423-0452]).

Those layers answer a different question than runtime policy. `.orbitignore` decides whether a path enters `ctx.file_paths`; policy `denyRead` / `denyModify` decides whether a later tool call may read or modify that path. One is graph inclusion, the other runtime filesystem access.

That split is structural on purpose. The knowledge-graph crate depends only on `orbit-common`; it does not depend on `orbit-policy`, and the scanner does not consult runtime policy while building the graph. This keeps graph refresh deterministic and keeps policy semantics out of the indexing hot path.

The default `.orbitignore` baseline excludes common generated or runtime-owned trees (`.orbit/`, `node_modules/`, `target/`, `dist/`, `build/`, virtualenvs, caches, `*.egg-info/`). `orbit workspace init` seeds the list so users can edit it. A path can appear in both layers with different intent: benchmark transcripts may be excluded from indexing while policy still allows reads; `.orbit/` may be excluded from indexing while policy denies modification.

Both scan paths use directory-entry file type metadata rather than `Path::is_dir()` / `Path::is_file()`, so symlinks are classified as symlinks instead of followed. The scanner recurses only into non-symlink directories and records only regular files; `.orbitignore` discovery uses the same boundary. That prevents repository symlinks from pulling in files outside the workspace or creating recursive scan cycles ([T20260509-33]).

### 2.5 Build benchmark scoreboard

`make bench` runs the `orbit-knowledge` `examples/graph_build.rs` driver as an end-to-end pipeline benchmark ([T20260426-0236]). It calls `pipeline::run_build` directly, measuring pipeline plus object persistence without CLI dispatch overhead.

Each invocation runs two scenarios against the workspace root by default:

- **`cold_build`** — delete `.orbit/knowledge/`, run a full non-incremental build, and record wall time, best-effort peak RSS, file count, leaf count, and dir count.
- **`warm_incremental_noop`** — reuse the cold build output, run an incremental build with no file changes, and record the same metrics.

Results append to `.orbit/state/scoreboard/graph_bench.json` as a 200-record capped JSON array with timestamp, git SHA, host/core context, and per-scenario metrics. It is local trend data, not a CI gate.

---

## 3. Query Surface

Canonical read behavior goes through `orbit_knowledge::commands::*` ([T20260510-5]). Tool adapters parse envelopes and shape JSON, while commands own query semantics such as search ranking, overview format selection, reference classification, SQLite fast paths, and service fallback. Lower-level services still wrap loaded graph snapshots and provide reusable primitives for the commands.

### 3.1 Selectors

A `Selector` is the universal addressing primitive. It accepts:

- a bare location (`crates/orbit-common/src/types/error.rs`)
- a location + kind disambiguator (`crates/orbit-common/src/types/error.rs:OrbitError:enum`) for distinguishing kind-specific selectors at the same qualified name
- a node id directly

All tool inputs that reference a node accept a selector string.

Symbol selectors keep the same `symbol:{path}#{qualified_name}:{kind}` envelope, but the `qualified_name` portion is language-owned and opaque. The leaf-ID uniqueness scheme may add class scope, Rust impl qualifiers, overload arity, or occurrence suffixes inside that portion; callers should rediscover affected selectors through graph query results rather than constructing method/impl/overload selectors by hand.

### 3.2 Services

| Command | Lower-level primitive | Notable task |
|---------|-------------|--------------|
| Overview | `GraphContextService::overview`, `GraphIndexReader` | Compact mode above 50 files added in [T20260412-0645-2]; broad summaries use the SQLite sidecar when current ([T20260509-72]); `top_files` is a bounded min-heap top-K rather than collect-and-sort ([T20260509-68]) |
| Search | `search_hits_with_total_bounded`, `GraphIndexReader` | Source-regex and structured selector search; default ranking lives in `commands::search`; exact-name and path-prefix queries take the SQLite fast path when the sidecar is current ([T20260509-73]); SQL/fallback equivalence is enforced as a multiset over leaves after [T20260510-1] and [T20260510-7] |
| Context | `bounded_context(selector, budget)`, `GraphIndexReader` | — |
| References | `find_references(selector)` | `commands::refs` owns code/doc/config classification |
| Callers | `transitive_callers(selector, depth)` | [T20260412-0645-3] |
| Implementors | `trait_implementors(trait)` | [T20260412-0645-3] |
| Dependencies | `crate_dependencies(crate)` | [T20260412-0645-3] |
| Lineage | `render_lineage_pack(selector)` | — |
| Show | `read_node_by_id`, `GraphIndexReader` | Selector lookup goes through the SQLite unique-selector index and hydrates a single node rather than calling `read_graph` ([T20260509-71], [T20260509-74]); fallback path also patches `children` to use `LeafNode.children` so SQL and JSON return identical child lists ([T20260510-2]) |
| Pack | `pack_json(...)` | Agent-friendly field projection ships in the same era as [T20260411-0424]; selector packing is bounded and prompt-first by default after [T20260505-5] |

All command-backed reads are against a resolved snapshot. Graph mutation code remains internal/deferred; the current public surface does not expose graph write tools.

Search no longer accepts a `task_id` filter. Task-to-commit lookup is handled by `git log --grep '[T...]'`; the graph remains focused on code structure and source queries.

Default `orbit.graph.search` ranking keeps a bounded candidate pool before ranking rather than retaining every matching node. The cap gives ranking enough headroom over the requested `limit` while preventing broad searches from allocating unbounded hit sets on large graphs ([T20260509-67]).

### 3.3 Object/blob read cache

`KnowledgeStore` owns a bounded `GraphObjectCache` for selector-oriented reads ([T20260426-0141]). The cache keeps two LRU sets: graph node objects keyed by object hash and source blobs keyed by blob hash. Default capacities are 10,000 objects and 2,000 blobs, enforced by the `lru` crate.

Object and blob hashes are content-addressed, so a cache hit can trust the value already verified on insertion. `read_graph_object` and `extract_leaf_source` verify SHA-256 integrity only on cache miss, then insert the verified value. The cache is scoped to a `KnowledgeStore` instance rather than a global singleton so separate workspaces and tests cannot cross-contaminate.

### 3.4 Source hydration is opt-in on broad graph reads

`GraphObjectStore::read_graph` accepts `GraphReadOptions` with separate `hydrate_file_source` and `hydrate_leaf_source` flags, both defaulting to `false` ([T20260509-65]). The object store still loads node objects and preserves `source_blob_hash`, but it leaves `FileNode.source` and `LeafNode.source` empty unless the caller explicitly asks for the matching source class.

The tool surface maps that choice to actual query needs: `show` hydrates file and leaf source; `refs`, `callers`, and `implementors` hydrate leaf source; `search` hydrates both only for `source_regex`; `overview`, default search, `deps`, and the `history` compatibility stub do not hydrate source. `pack` keeps summary mode body-free and hydrates leaf source only when `summary: false`. Incremental rebuild reuse still hydrates both file and leaf source because it copies unchanged snapshots into the next graph.

---

## 4. Orbit Integration

### 4.1 CLI

```
orbit graph build [--ref <name>]
orbit graph update [--ref <name>]
orbit graph show --ref <name> <selector>
orbit graph search --ref <name> <query>
```

`show`/`search` subcommands and the `leaf` → `symbol` vocabulary rename landed under [T20260411-0424].

Graph build/update writes without `--ref` resolve the current git branch and fail on detached HEAD rather than inventing a label. Reads fall back to the default branch when the current-branch ref is missing, with one stderr warning ([T20260421-0358]).

The CLI does not import `orbit-knowledge` directly. `orbit-tools::graph` owns build/update, show/search, the `graph history` compatibility stub, and the default `.orbitignore` template; `orbit-core::command::graph` re-exports that facade for clap and workspace init ([T20260426-2042]). CLI and agent tools therefore share JSON payload builders.

### 4.2 MCP tools

The knowledge graph is exposed through `orbit-mcp` as a stable, read-only tool surface. Each tool accepts an optional `ref` and delegates to the services above:

- `orbit.graph.overview`
- `orbit.graph.search`
- `orbit.graph.show`
- `orbit.graph.pack`
- `orbit.graph.callers`
- `orbit.graph.implementors`
- `orbit.graph.refs`
- `orbit.graph.history` (compatibility stub that reports graph task attribution removal and points to `git log --grep '[T<task-id>]'`)

An explicit `ref` means "read the stored graph for this ref." It does not trigger a rebuild against whatever branch is currently checked out, and it does not overlay task-local working-graph edits onto that explicit historical ref. That is a subtle but load-bearing rule for historical queries.

`orbit.graph.pack` optimizes for the agent context-gathering path. It reads the existing graph snapshot by default instead of starting an inline auto-refresh, and returns an `auto_refresh.skipped` diagnostic with remediation guidance. Callers that deliberately want the old inline refresh path pass `refresh: true`. The packer also accepts `timeout_ms` (default 15000) for selector projection; when the budget is exhausted between selectors, remaining selectors are returned as unresolved entries with timeout hints rather than withholding all results ([T20260505-5]).

### 4.3 Activity interaction

Activities use graph tools for inspection and prompt assembly only. Code-mutating workflows coordinate before dispatch in `task_gate_pipeline`: the deterministic `reserve_locks` activity reserves task `context_files` as the preflight guard, then optimistic integration/review checks catch stale or overlapping edits. Activities that only read query the service directly.

The working graph still exists in `crates/orbit-knowledge/src/working_graph`, but it is not a public agent-facing mutation API in this version. Branch-local graph refs are intentionally not used as distributed locks because agents commonly work in separate worktrees with separate refs.

---

## 5. Locking

Nodes carry `is_locked`, `lineage_locked`, `lock_owner`, and `lock_reason` on `BaseNodeFields`. Two lock shapes matter:

- **`is_locked`** — the node itself is frozen. Attempts to mutate it in the working graph produce a `WriteError`.
- **`lineage_locked`** — the node's identity is frozen. Renames/re-identifications across builds are blocked; the identity key must survive rebuild.

Graph-node locks survive rebuilds because they live on the node body (and therefore in the content-addressed object store), but they are not the current public write-admission mechanism. Public workflow coordination happens at the task plane when `task_gate_pipeline` runs the `reserve_locks` activity before dispatch.

The shared file-based lock store was introduced in [T20260411-0424] (replacing a removed `orbit-lock` crate) and further hardened in [T20260417-0301-2].

---

## 6. Concerns & Honest Limitations

### 6.1 Extractor coverage drives graph precision

The leaf graph is only as good as each extractor. Code coverage (tree-sitter): C, C#, Rust, Python, Go, Java, JavaScript, Kotlin, TypeScript, TSX, and Ruby. Doc coverage: markdown (ATX headings only — no frontmatter, no fenced-block leaves, no RST). Config (YAML / JSON / TOML) and table (CSV / TSV) files now land as leafless `FileNode`s after [T20260509-64] — file-level search and `show` still resolve via file source, but per-key and per-column leaves are no longer emitted. Anything outside those families (shell scripts, C++, `.env`, SQL, Razor, etc.) similarly lands as a leafless `FileNode`, and the agent has to fall back to file-level reads for the uncovered slice. Depth-of-extraction tradeoffs are captured in ADR-003, ADR-011, and ADR-038.

### 6.2 No cross-file reference resolution

The service computes callers and implementors from extracted signatures, not type-resolved references ([T20260412-0645-3]). A method call and same-named variable deref can look identical, so `find_references` is a superset of the truth. Safety-critical refactors still require verification.

### 6.3 Historical attribution was removed

Node-level task attribution was removed in [T20260506-11]. The graph no longer answers "which tasks touched this selector"; use `git log --grep '[T...]'` for local forward lookup from task to commits, and use `external_refs` for cross-engineer task references.

### 6.4 The graph is a snapshot, not a stream

Queries answer "what does this branch look like as of the last build." Public file edits are stale relative to the graph until the next refresh; deferred/internal working-graph mutations are the only path that can model mid-edit state. `ensure_fresh` narrows the gap but does not close it — a read right after an external edit and before a debounce window elapses can return pre-edit results. `orbit.graph.pack` is intentionally more conservative than the other read tools: it skips inline refresh by default so task context selection cannot disappear into a rebuild without prompt-visible timeout guidance ([T20260505-5]).

### 6.5 Branch-scoped refs do not solve worktree-local reads

Two worktrees on the same branch share a ref ([T20260421-0358]). If one worktree is mid-rebuild and the other reads, the reader sees the pre-rebuild snapshot until the atomic rename publishes the new ref. This is fine and arguably correct, but worth naming.

### 6.6 Manifest and ref can drift under manual intervention

The manifest is a consistency hint, not an authoritative oracle. A user who deletes `refs/heads/<branch>.json` but leaves the manifest and index on disk creates an inconsistent state. Orbit's migration code handles the documented legacy case (`refs/current.json`) but not arbitrary hand-edits. The recovery path — as exercised by [T20260416-0719] — is to delete the manifest and force a full rebuild.

### 6.7 No garbage collection

Objects and blobs accumulate forever. There is no `orbit graph gc` today — abandoned branch refs still pin their indexes, and reachable sets have to be walked before deletion is safe. At current repo sizes this is not a problem; at workspace scale it will be.

### 6.8 Task visibility is outside the graph

The removed attribution index never distinguished shipped, reverted, or WIP task signal. Future task visibility should be designed through task sync, task state, or external trackers rather than by restoring graph node fields.

### 6.9 Object/blob cache lifetime is store-scoped

The read cache is per `KnowledgeStore`, not global ([T20260426-0141]). Long-running services that retain a store instance benefit across repeated selector reads, and tests get simple isolation. Short-lived CLI invocations that open a fresh store for each process do not share cache entries across process boundaries.

### 6.10 Parallel build stages depend on ordered reassembly

`compute_hashes` and `build_graph_leaves` use worker threads for per-file work, but content-addressed graph stability depends on reassembling results in canonical scan/file order ([T20260426-0139]). Any future stage that pushes directly from workers into `ctx.graph.leaves`, `FileNode.leaf_children`, or serialized hash output would make root object hashes scheduler-dependent.

### 6.11 Benchmark numbers are trend data, not portable truth

`graph_bench.json` records wall time and peak RSS from the local machine that ran `make bench` ([T20260426-0236]). The default corpus is the Orbit repo itself, so counts and timings move with checked-in source and benchmark fixtures. Compare records as local trend signals and include the machine/core context when using them in reviews.

### 6.12 Symlinked source trees are excluded

The scanner skips symlinked directories rather than trying to canonicalize and follow only in-workspace targets ([T20260509-33]). This is the safer default for agent indexing because it avoids outside-repo leakage and cycles, but a workspace that intentionally exposes source through symlinked directories must materialize those files or wait for an explicit, cycle-safe opt-in traversal policy.

### 6.13 SQLite secondary index coverage is intentionally partial

The `graph/graph_index.sqlite` sidecar exists to make selector, name, and file-symbol-count reads sublinear ([T20260509-70]). Readers must verify `meta.graph_ref` against the resolved branch ref before trusting it; missing, stale, corrupt, or unsupported shapes fall back to the JSON by-id index and object store. As of [T20260509-72], broad `overview` summaries query aggregate counts and top files from SQLite, but scoped summaries still fall back so their output stays byte-for-byte aligned with the established prefix semantics.

---

## Task References

- **[T20260411-0008]** — Extract `orbit-knowledge` crate from `orbit-tools`.
- **[T20260411-0424]** — Consolidate `orbit-knowledge`; prototype graph editing internals (`add`/`delete`/`move`); introduce shared file-based lock store; add `show`/`search` CLI; rename `leaf` to `symbol` at the tool surface.
- **[T20260412-0645-2]** — Compact `orbit.graph.overview` output for large repos.
- **[T20260412-0645-3]** — Architectural graph navigation: `deps`, `implementors`, `callers`.
- **[T20260416-0719]** — Recover from a corrupted default knowledge graph store.
- **[T20260417-0301-2]** — Harden graph lock/write/read tooling.
- **[T20260417-0307]** — Gate and guard graph refresh/search hot paths.
- **[T20260417-0639]** — Speed up workspace-init graph persistence hot path.
- **[T20260421-0358]** — Scope graph refs by branch.
- **[T20260421-0528]** — Historical `task_ids` schema on every node + git history walker for attribution; removed by [T20260506-11].
- **[T20260422-1540]** — Extend extraction to markdown sections, top-level config keys, and CSV/TSV columns via `FileKind`-dispatched extractors (`FileExtractor` trait, replacing `LanguageExtractor`).
- **[T20260423-0452]** — `.orbitignore` scan exclusions, nested composition, default ignore baseline, and workspace-init seeding.
- **[T20260426-0139]** — Parallelize per-file hashing and leaf extraction while preserving deterministic graph output.
- **[T20260426-0140]** — Reuse prior file and leaf snapshots for unchanged paths during incremental graph rebuilds.
- **[T20260426-0141]** — Bounded `KnowledgeStore` LRU for graph objects and source blobs.
- **[T20260426-0220]** — Historical exact `task_id` filtering in `orbit.graph.search`; removed by [T20260506-11].
- **[T20260426-0236]** — Add `make bench` graph build benchmark and `.orbit/state/scoreboard/graph_bench.json`.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface; use task lock reservations as preflight write guards.
- **[T20260426-2042]** — Move graph CLI behavior behind the `orbit-tools::graph` facade and remove the direct `orbit-knowledge` dependency from `orbit-cli`.
- **[T20260428-1]** — Historical graph task-ID attribution/search alignment; superseded by [T20260506-11].
- **[T20260430-22]** — Compact the knowledge-graph design docs and remove duplicate top-level narrative.
- **[T20260505-11]** — Add TypeScript and TSX extraction coverage, documented by gpt-5.5.
- **[T20260505-13]** — Add C# extraction coverage, documented by gpt-5.5.
- **[T20260505-16]** — Add C and header extraction coverage, documented by gpt-5.
- **[T20260505-5]** — Bound `orbit.graph.pack` selector gathering and skip inline refresh by default, documented by gpt-5.5.
- **[T20260506-11]** — Remove graph task attribution after 0/961 audited reverse-lookup uses; preserve task IDs as local commit-search keys.
- **[T20260509-33]** — Skip symlinked directories/files during knowledge scans and `.orbitignore` discovery to prevent outside-repo indexing and recursive cycles.
- **[T20260509-64]** — Collapse YAML/JSON/TOML/CSV/TSV extraction to file-as-leaf; keep `FileKind::Config(_)` / `FileKind::Table(_)` classification.
- **[T20260509-65]** — Add `GraphReadOptions` so broad graph reads skip file/leaf source hydration unless a tool opts in.
- **[T20260509-68]** — Replace `overview.top_files` Vec-then-sort with a bounded min-heap top-K.
- **[T20260509-70]** — Build the write-only SQLite secondary index sidecar during graph persistence.
- **[T20260509-71]** — Add the read-side `GraphIndexReader` facade with version check and fallback.
- **[T20260509-72]** — Use the SQLite secondary index for current, unscoped `orbit.graph.overview` summary aggregation.
- **[T20260509-73]** — Wire exact-name and path-prefix `orbit.graph.search` queries through the SQLite sidecar.
- **[T20260509-74]** — Wire `orbit.graph.show` selector resolution through the SQLite unique-selector index.
- **[T20260510-1]** — Restore SQL/fallback equivalence for `orbit.graph.search` (substring on either column).
- **[T20260510-2]** — Restore SQL/fallback equivalence for `orbit.graph.show` `children` (use forward leaf pointers).
- **[T20260510-7]** — Specify per-file leaf-ID uniqueness via language-natural qualifiers plus deterministic occurrence suffixes.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
