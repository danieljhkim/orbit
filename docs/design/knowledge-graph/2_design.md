# Knowledge Graph — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26 (parallel build stages, task-id search, and graph build benchmark, [T20260426-0139], [T20260426-0220], [T20260426-0236])

This document specifies the knowledge graph as it exists today: on-disk layout, build pipeline, query services, Orbit integration, locking, and honest limitations. See [1_overview.md](./1_overview.md) for the "why" and [3_vision.md](./3_vision.md) for where it is headed. Task IDs are cited inline and collected at the end.

---

## 1. On-Disk Layout

```
.orbit/knowledge/
├── manifest.json                commit pointer + generated_at timestamp
├── refresh.lock                 flock for single-flighted rebuilds
├── refresh_state.json           debounce bookkeeping for dirty refreshes
├── graph/
│   ├── objects/<hh>/<hash>.json   immutable node bodies (dir / file / leaf)
│   ├── blobs/<hh>/<hash>.txt      immutable source blobs
│   ├── index/by-id/<root-graph-hash>.json   immutable per-build index
│   └── refs/heads/<branch>.json   mutable branch ref → active index
└── working/<activity_id>/       (in memory today; see 3_vision.md §2.3)
```

The split between immutable objects/blobs/index and mutable refs is deliberate. Rebuilds always write new immutable artifacts and then atomically rename a ref file; no object file is ever overwritten. This survives concurrent worktree rebuilds and interrupted writes ([T20260421-0358]).

The legacy `refs/current.json` layout is migrated to `refs/heads/<default-branch>.json` on open; see [specs/refs.md](./specs/refs.md).

---

## 2. Build Pipeline

The pipeline is a straight-line sequence of plain functions operating on a shared `PipelineContext`:

```
scan → hash → detect_changes → build_dirs → build_files → build_leaves → attribute_history → persist → write_manifest → save_hash_cache
```

| Stage | Output | Notes |
|-------|--------|-------|
| `scan_repo` | `ctx.file_paths` | Walks the repo honoring `.gitignore` plus Orbit-specific `.orbitignore` rules |
| `compute_hashes` | Per-file content hashes | Drives incremental detection; reads and hashes files in parallel, then publishes `ctx.new_hashes` after the worker phase ([T20260426-0139]) |
| `detect_changes` | Added / modified / unchanged path set | Incremental leaf extraction uses this set to decide what can reuse the prior graph |
| `build_graph_dirs` | `DirNode` entries with parent/child wiring | Deterministic; root dir id derived from `.` |
| `build_graph_files` | `FileNode` entries linked to parent dir | Language detected from extension |
| `build_graph_leaves` | `LeafNode` entries via file-kind-dispatched extractor | Code via tree-sitter (Rust, Python, Go, Java, JavaScript); markdown sections, YAML/JSON/TOML top-level keys, and CSV/TSV header columns via shallow extractors added in [T20260422-1540]. Per-file read/extract work runs in parallel, then graph mutation is merged on the main thread in file order ([T20260426-0139]). Incremental builds reuse unchanged file/leaf snapshots from the same branch ref when hashes match ([T20260426-0140]) |
| `attribute_history` | `task_ids` on touched nodes | Introduced in [T20260421-0528] |
| `persist_graph` | Content-addressed objects, blobs, index | Atomic via tempfile + rename |
| `write_manifest` | `manifest.json` | Timestamp + commit + ref pointer |
| `save_hash_cache` | `hashes.json` | Baseline for the next incremental `detect_changes` pass |

Extraction is dispatched on `FileKind`. Each `FileExtractor` (renamed from `LanguageExtractor` in [T20260422-1540]) emits `ExtractedLeaf` records with a `qualified_name`, `kind`, source span, source hash, and child qualified names (methods inside classes, associated fns inside impls). Code extractors wrap tree-sitter grammars; the shallow doc/config/table extractors parse their formats directly (ATX headings for markdown, top-level map entries for YAML/JSON/TOML via the existing serde ecosystem, first-row cells for CSV/TSV with a 1 MiB size cap).

Hashing and leaf extraction are parallelized only across independent file work. `compute_hashes` collects `(path, sha256)` worker results before replacing `ctx.new_hashes`; `build_graph_leaves` workers return either a reusable prior snapshot or freshly extracted file output, and the main thread applies those outputs sorted by original `FileNode` index. That merge discipline keeps `ctx.graph.leaves` and each `FileNode.leaf_children` byte-stable relative to the old sequential order while avoiding locks around `PipelineContext` ([T20260426-0139]). Individual file read failures remain non-fatal skips in both phases.

### 2.1 Incremental refresh

`ensure_fresh(knowledge_dir, repo_path)` is the read-side entry point. It compares the persisted manifest against the current HEAD timestamp and the dirty-worktree fingerprint, and picks one of:

- **Fresh** — nothing to do.
- **SkippedDirtyDebounce** — worktree is dirty with the same fingerprint as a recent refresh; reuse.
- **Rebuild** — incremental if a manifest exists, full otherwise.
- **SkippedConcurrent** — another process already holds the refresh lock; wait briefly and reuse its result.

The refresh lock is a `flock` on `.orbit/knowledge/refresh.lock`. Concurrent callers either wait for the in-flight build or fall through to the pre-existing graph once the lock-holder publishes a new ref. The debounce window is `ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS` (default 5s).

Incremental rebuilds still rebuild the directory and file node skeleton from the current scan so deletes and `.orbitignore` changes take effect. The expensive leaf phase loads the previously persisted graph for the same branch ref and copies unchanged files' `source_blob_hash`, hydrated source, exports, re-exports, `leaf_children`, and leaf nodes when both the file source hash and every reused leaf's `file_hash_at_capture` match the new hash. Paths in `ctx.changed_paths`, new files, hash mismatches, absent refs, or unreadable prior graphs take the full extractor path instead ([T20260426-0140]).

Refresh hardening: [T20260417-0307] gated and guarded the refresh/search hot paths; [T20260416-0719] added recovery from a corrupted default store; [T20260417-0639] sped up the workspace-init persistence path.

### 2.2 Attribution pass

After the structural build, the history walker (`pipeline::history`, [T20260421-0528]) runs:

1. Resolve HEAD → last-attributed commit from the previous ref (or full history on first build).
2. `git log --reverse --topo-order` over the new commits.
3. Per commit: parse task IDs (regex `\[T\d{8}-\d{4}(?:-\d+)?\]`), diff against first parent (`--unified=0 --first-parent -m`), map hunks to leaves by line-range overlap.
4. Union task IDs onto touched leaves; set `structural_conflict` on two-sided merges.
5. Write `last_attributed_commit` back into the ref.

The walker shells out to `git` via `orbit_common::utility::git::run_git`. There is no in-process git library dependency — operational simplicity and behavioral parity with the CLI outweigh the per-commit process cost at current repo sizes.

### 2.3 Inclusion vs Access: `.orbitignore` vs Policy

The scan stage now evaluates two inclusion layers before a file ever reaches the parser:

1. Git's own ignore rules via `git check-ignore --stdin`.
2. Orbit's scan-only `.orbitignore` matcher inside `orbit-knowledge`, implemented with the `ignore` crate and composed from built-in defaults plus any root or nested `.orbitignore` files ([T20260423-0452]).

Those layers answer a different question than runtime policy. `.orbitignore` lives in `orbit-knowledge` and is evaluated at parse/scan time to decide whether a path becomes part of `ctx.file_paths` at all. Policy `denyRead` / `denyModify` lives in `orbit-policy` and is evaluated later, at tool-call time, when an activity asks to read or modify a path on disk. One is about graph inclusion; the other is about runtime filesystem access.

That split is structural on purpose. The knowledge-graph crate depends only on `orbit-common`; it does not depend on `orbit-policy`, and the scanner does not consult runtime policy while building the graph. This keeps graph refresh deterministic and keeps policy semantics out of the indexing hot path.

The default `.orbitignore` baseline excludes common generated or runtime-owned trees (`.orbit/`, `node_modules/`, `target/`, `dist/`, `build/`, `.venv/`, `venv/`, `__pycache__/`, `*.egg-info/`). `orbit workspace init` seeds the same list into a visible workspace-root `.orbitignore` file so users can edit the defaults without discovering the behavior by reading source first.

The same path may legitimately appear in both layers with different intent. For example, `benchmarks/graph_v1/runs/**` is excluded from graph indexing via `.orbitignore` so frozen benchmark transcripts do not pollute `orbit.graph.search`, while a policy profile could still allow or deny runtime reads to that subtree depending on the activity. Likewise, `.orbit/` is excluded from indexing because it is runtime state, and a policy may independently deny modification of `.orbit/**` during normal agent execution.

### 2.4 Build benchmark scoreboard

`make bench` runs the `orbit-knowledge` example driver `examples/graph_build.rs` as an end-to-end benchmark for the build pipeline ([T20260426-0236]). The driver calls `pipeline::run_build` directly rather than shelling out to `orbit graph build`, so it measures the pipeline plus object persistence without CLI dispatch overhead.

Each invocation runs two scenarios against the workspace root by default:

- **`cold_build`** — delete `.orbit/knowledge/`, run a full non-incremental build, and record wall time, best-effort peak RSS, file count, leaf count, and dir count.
- **`warm_incremental_noop`** — reuse the cold build output, run an incremental build with no file changes, and record the same metrics.

Results append to `.orbit/state/scoreboard/graph_bench.json` as an ordered JSON array capped at 200 records. Each record includes timestamp, git SHA, hostname, logical core count, and the per-scenario metrics. The scoreboard is a developer-machine trend aid, not a CI gate; shared runners are too noisy for absolute thresholds, and the default corpus grows as the repo grows.

---

## 3. Query Surface

Reads go through `GraphContextService`, which wraps a `GraphNavigator` over a loaded `CodebaseGraphV1` and layers selector resolution on top.

### 3.1 Selectors

A `Selector` is the universal addressing primitive. It accepts:

- a bare location (`crates/foo/src/lib.rs`)
- a location + kind disambiguator (`crates/foo/src/lib.rs:Foo:struct`) for distinguishing struct-vs-impl at the same qualified name
- a node id directly

All tool inputs that reference a node accept a selector string.

### 3.2 Services

| Service | Entry point | Notable task |
|---------|-------------|--------------|
| Overview | `overview(prefix?)` | Compact mode above 50 files added in [T20260412-0645-2] |
| Search | `search(query)` / `search_structured` | `task_id` filtering exposed through `orbit.graph.search` in [T20260426-0220] |
| Context | `bounded_context(selector, budget)` | — |
| References | `find_references(selector)` | — |
| Callers | `transitive_callers(selector, depth)` | [T20260412-0645-3] |
| Implementors | `trait_implementors(trait)` | [T20260412-0645-3] |
| Dependencies | `crate_dependencies(crate)` | [T20260412-0645-3] |
| Lineage | `render_lineage_pack(selector)` | — |
| Pack | `pack_json(...)` | Agent-friendly field projection ships in the same era as [T20260411-0424] |

All services are read-only against a resolved snapshot. Writes go through the working graph, never through the service.

Search accepts an optional task-id filter that exact-matches against the `task_ids` vector stored on every node ([T20260426-0220]). The filter composes with query text, node type, kind, prefix, and source regex by logical AND. When present, it is applied before source-regex matching so task-scoped regex searches do not spend their candidate budget on unrelated nodes. Missing or null `task_id` preserves the pre-existing search behavior.

### 3.3 Object/blob read cache

`KnowledgeStore` owns a bounded `GraphObjectCache` for selector-oriented reads ([T20260426-0141]). The cache keeps two LRU sets: graph node objects keyed by object hash and source blobs keyed by blob hash. Default capacities are 10,000 objects and 2,000 blobs, enforced by the `lru` crate.

Object and blob hashes are content-addressed, so a cache hit can trust the value already verified on insertion. `read_graph_object` and `extract_leaf_source` verify SHA-256 integrity only on cache miss, then insert the verified value. The cache is scoped to a `KnowledgeStore` instance rather than a global singleton so separate workspaces and tests cannot cross-contaminate.

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

Writes without `--ref` resolve the current git branch; writes fail on detached HEAD rather than inventing a branch label. Reads fall back to the default branch when the current-branch ref does not yet exist, with a single stderr warning ([T20260421-0358]).

### 4.2 MCP tools

The knowledge graph is exposed through `orbit-mcp` as a stable tool surface. Each tool accepts an optional `ref` and delegates to the services above:

- `orbit.graph.overview`
- `orbit.graph.search` (optional `task_id`, validated as `T\d{8}-\d{4}`)
- `orbit.graph.show`
- `orbit.graph.pack`
- `orbit.graph.callers`
- `orbit.graph.implementors`
- `orbit.graph.refs`
- `orbit.graph.add` / `orbit.graph.delete` / `orbit.graph.move` (working-graph mutators, [T20260411-0424])

An explicit `ref` means "read the stored graph for this ref." It does not trigger a rebuild against whatever branch is currently checked out, and it does not overlay task-local working-graph edits onto that explicit historical ref. That is a subtle but load-bearing rule for historical queries.

### 4.3 Activity interaction

Activities that mutate code load the working graph at start, apply edits through it, and either commit (merging into the branch snapshot and triggering an incremental rebuild) or discard on rollback. Activities that only read skip the working graph entirely and query the service directly.

---

## 5. Locking

Nodes carry `is_locked`, `lineage_locked`, `lock_owner`, and `lock_reason` on `BaseNodeFields`. Two lock shapes matter:

- **`is_locked`** — the node itself is frozen. Attempts to mutate it in the working graph produce a `WriteError`.
- **`lineage_locked`** — the node's identity is frozen. Renames/re-identifications across builds are blocked; the identity key must survive rebuild.

Locks are set explicitly via tools and survive rebuilds because they live on the node body (and therefore in the content-addressed object store). A locked node keeps its lock across branches until explicitly released.

The shared file-based lock store was introduced in [T20260411-0424] (replacing a removed `orbit-lock` crate) and further hardened in [T20260417-0301-2].

---

## 6. Concerns & Honest Limitations

### 6.1 Extractor coverage drives graph precision

The leaf graph is only as good as each extractor. Code coverage (tree-sitter): Rust, Python, Go, Java, JavaScript. Doc coverage: markdown (ATX headings only — no frontmatter, no fenced-block leaves, no RST). Config coverage: YAML / JSON / TOML top-level keys only — nested paths like `a.b.c` are not indexed. Tabular coverage: CSV / TSV header row only; files over 1 MiB produce zero leaves by design. Anything outside those families (shell scripts, C/C++, `.env`, SQL, etc.) lands in the graph as a leafless `FileNode`, and the agent has to fall back to file-level reads for the uncovered slice. Depth-of-extraction tradeoffs are captured in ADR-011.

### 6.2 No cross-file reference resolution

The service computes callers and implementors from extracted symbol signatures, not from type-resolved references ([T20260412-0645-3]). A method call and a same-named variable deref look the same to the current indexer. This is a pragmatic choice — full type resolution would require a per-language type checker — but it means `find_references` is a superset of the truth, not the truth itself. Agents relying on callers/implementors for safety-critical refactors should verify.

### 6.3 History attribution is best-effort

Hunk-to-symbol mapping uses line-range overlap against the symbol's span *at the commit's tree* ([T20260421-0528]). Renames, moves, and reformatting confuse this. The walker deliberately does not try to track renames through `--follow` because the line-mapping cost compounds with every rename hop; we accept that a symbol moved across files gets attribution only from the post-move commits.

### 6.4 The graph is a snapshot, not a stream

Queries answer "what does this branch look like as of the last build." Mid-edit state lives in the working graph; anything outside the working graph is stale relative to the filesystem until the next refresh. `ensure_fresh` narrows the gap but does not close it — a read right after an external edit and before a debounce window elapses can return pre-edit results.

### 6.5 Branch-scoped refs do not solve worktree-local reads

Two worktrees on the same branch share a ref ([T20260421-0358]). If one worktree is mid-rebuild and the other reads, the reader sees the pre-rebuild snapshot until the atomic rename publishes the new ref. This is fine and arguably correct, but worth naming.

### 6.6 Manifest and ref can drift under manual intervention

The manifest is a consistency hint, not an authoritative oracle. A user who deletes `refs/heads/<branch>.json` but leaves the manifest and index on disk creates an inconsistent state. Orbit's migration code handles the documented legacy case (`refs/current.json`) but not arbitrary hand-edits. The recovery path — as exercised by [T20260416-0719] — is to delete the manifest and force a full rebuild.

### 6.7 No garbage collection

Objects and blobs accumulate forever. There is no `orbit graph gc` today — abandoned branch refs still pin their indexes, and reachable sets have to be walked before deletion is safe. At current repo sizes this is not a problem; at workspace scale it will be.

### 6.8 Task-ID attribution mixes review-ready and WIP signal

`task_ids` on a node today is a flat union ([T20260421-0528]). A node touched by a reverted task and by a shipped task shows both IDs with no way to distinguish "this change is live" from "this change was tried and backed out." Consumers that want shipped-only signal have to join against task status externally.

### 6.9 Object/blob cache lifetime is store-scoped

The read cache is per `KnowledgeStore`, not global ([T20260426-0141]). Long-running services that retain a store instance benefit across repeated selector reads, and tests get simple isolation. Short-lived CLI invocations that open a fresh store for each process do not share cache entries across process boundaries.

### 6.10 Parallel build stages depend on ordered reassembly

`compute_hashes` and `build_graph_leaves` use worker threads for per-file work, but content-addressed graph stability depends on reassembling results in canonical scan/file order ([T20260426-0139]). Any future stage that pushes directly from workers into `ctx.graph.leaves`, `FileNode.leaf_children`, or serialized hash output would make root object hashes scheduler-dependent.

### 6.11 Benchmark numbers are trend data, not portable truth

`graph_bench.json` records wall time and peak RSS from the local machine that ran `make bench` ([T20260426-0236]). The default corpus is the Orbit repo itself, so counts and timings move with checked-in source and benchmark fixtures. Compare records as local trend signals and include the machine/core context when using them in reviews.

---

## Task References

- **[T20260411-0008]** — Extract `orbit-knowledge` crate from `orbit-tools`.
- **[T20260411-0424]** — Consolidate `orbit-knowledge`; add graph editing tools (`add`/`delete`/`move`); introduce shared file-based lock store; add `show`/`search` CLI; rename `leaf` to `symbol` at the tool surface.
- **[T20260412-0645-2]** — Compact `orbit.graph.overview` output for large repos.
- **[T20260412-0645-3]** — Architectural graph navigation: `deps`, `implementors`, `callers`.
- **[T20260416-0719]** — Recover from a corrupted default knowledge graph store.
- **[T20260417-0301-2]** — Harden graph lock/write/read tooling.
- **[T20260417-0307]** — Gate and guard graph refresh/search hot paths.
- **[T20260417-0639]** — Speed up workspace-init graph persistence hot path.
- **[T20260421-0358]** — Scope graph refs by branch.
- **[T20260421-0528]** — `task_ids` schema on every node + git history walker for attribution.
- **[T20260422-1540]** — Extend extraction to markdown sections, top-level config keys, and CSV/TSV columns via `FileKind`-dispatched extractors (`FileExtractor` trait, replacing `LanguageExtractor`).
- **[T20260423-0452]** — `.orbitignore` scan exclusions, nested composition, default ignore baseline, and workspace-init seeding.
- **[T20260426-0139]** — Parallelize per-file hashing and leaf extraction while preserving deterministic graph output.
- **[T20260426-0140]** — Reuse prior file and leaf snapshots for unchanged paths during incremental graph rebuilds.
- **[T20260426-0141]** — Bounded `KnowledgeStore` LRU for graph objects and source blobs.
- **[T20260426-0220]** — Add exact `task_id` filtering to `orbit.graph.search`.
- **[T20260426-0236]** — Add `make bench` graph build benchmark and `.orbit/state/scoreboard/graph_bench.json`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
