# Knowledge Graph — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26 (ADR-019)

ADR-style log of non-obvious design choices behind the knowledge graph. Each entry names the decision, the context that forced it, what we chose, and what we traded away. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. See [1_overview.md](./1_overview.md) and [2_design.md](./2_design.md) for the corresponding implementation; [3_vision.md](./3_vision.md) tracks questions that may trigger future ADRs.

---

## ADR-001 — Content-addressed objects + mutable refs

**Status:** Accepted · 2026-04 · [T20260407-0222], [T20260411-0424]

**Context.** The graph has to survive crashes mid-rebuild, support concurrent reads during a rebuild, and deduplicate unchanged nodes across builds. A single mutable JSON file fails all three. The original content-addressing refactor landed in [T20260407-0222] (then under `orbit-agent`); the layout stabilized in its current shape during the `orbit-knowledge` consolidation [T20260411-0424].

**Decision.** Adopt a git-style split: immutable content-addressed objects under `objects/<hh>/<hash>.json`, immutable blobs under `blobs/<hh>/<hash>.txt`, immutable per-build index under `index/by-id/<root-graph-hash>.json`, and a mutable branch ref as the only pointer that changes.

**Consequences.**
- Atomic ref swaps via tempfile + rename make interrupted writes safe.
- Object dedup is free because identical content produces identical paths.
- Cost: no GC today — orphan objects accumulate (see [3_vision.md §1.10]).

---

## ADR-002 — Branch-scoped refs over a single shared ref

**Status:** Accepted · 2026-04 · [T20260421-0358]

**Context.** The original layout used `.orbit/knowledge/graph/refs/current.json` — one mutable ref shared across every branch and worktree. The last rebuild won globally. Multi-branch and multi-worktree workflows therefore saw graph reads for the wrong branch, and concurrent rebuilds raced on the single pointer.

**Decision.** Namespace refs by branch: `refs/heads/<branch>.json`. Reads resolve the current git branch; writes fail on detached HEAD rather than invent a label. Reads fall back to the default branch's ref with a stderr warning when the current-branch ref does not yet exist; writes never fall back.

**Consequences.**
- Two worktrees on different branches can rebuild concurrently without corruption.
- A new branch is immediately usable via fallback — no forced rebuild.
- Migration path: legacy `refs/current.json` is moved to `refs/heads/<default>.json` on open.
- Cost: two worktrees on the *same* branch still share a ref (see [2_design.md §6.5]).

---

## ADR-003 — Tree-sitter extractors over an LSP backend

**Status:** Accepted · 2026-04 · [T20260406-0455-3], [T20260416-0352]

**Context.** Reference resolution is strongest via a language server, but LSPs are stateful long-running processes tuned for interactive UX. Agent tools want bulk, structured, token-budgeted output and low lifecycle overhead. The Rust extractor landed first ([T20260406-0455-3], hardened in [T20260409-0550]); Go, Java, and JavaScript followed in [T20260416-0352].

**Decision.** Use tree-sitter grammars with per-language extractors (`rust`, `python`, `go`, `java`, `javascript`) producing structural symbols only. Defer cross-file reference resolution indefinitely. See [3_vision.md §1.1] for the open question of re-introducing LSP as a pluggable backend.

**Consequences.**
- Fast, deterministic extraction with no per-query process lifecycle.
- `find_references`, `callers`, `implementors` ([T20260412-0645-3]) are signature-matched, not type-resolved — a superset of the truth.
- Cost: extractor maintenance scales with languages supported.

---

## ADR-004 — Shell out to the `git` CLI instead of an in-process library

**Status:** Accepted · 2026-04 · [T20260421-0528]

**Context.** The history walker diffs every new commit against its first parent, parses unified diffs, and resolves trees. An in-process git library (`gix`, `git2`) would avoid per-commit fork cost.

**Decision.** Shell out to `git` via `orbit_common::utility::git::run_git`. No in-process git dependency.

**Consequences.**
- Behavior matches what a user sees on the command line — trivially reproducible.
- No linked library adds build surface or ABI risk.
- Cost: per-commit fork overhead. Tolerable at current repo sizes; revisit if it shows up in refresh profiles.

---

## ADR-005 — Working graph is in-memory, not persistent

**Status:** Accepted (with open question) · 2026-04 · [T20260411-0424], [T20260409-0656], [T20260416-0236], [T20260417-0302]

**Context.** Activities mutate the graph as they edit code. Those mutations must not perturb the persisted store mid-turn (cache stability, concurrent-reader safety). Two implementations are plausible: in-memory overlay vs. per-activity disk staging. Working-graph edit buffering, version chains, and insertion support landed in [T20260409-0656]; write-anchor validation and atomicity guarantees followed in the [T20260416-0236] series (`-2` conflict/audit, `-3` canonical selectors, `-4` atomic moves). Source-file durability on edit ops was added in [T20260417-0302].

**Decision.** Keep the working graph in memory for the duration of an activity. Persist at activity boundaries only.

**Consequences.**
- Branch ref stays byte-stable inside an activity — queries are reproducible.
- Zero disk churn for reads-only activities.
- Cost: a crashed long activity loses its staging. Recovery = rerun. See [3_vision.md §1.3].

---

## ADR-006 — Hunk-to-symbol attribution by line-range overlap only

**Status:** Accepted · 2026-04 · [T20260421-0528]

**Context.** `git log --follow` chases renames through history but at non-trivial per-hop cost. Hunk coordinates have to be re-mapped after every rename hop. At commit volumes typical of this repo, follow mode compounds into minutes of extra walker time.

**Decision.** Map hunks to leaves by line-range overlap against the symbol's span *at the commit's tree*. Do not chase renames. A symbol moved across files gets attribution from post-move commits only.

**Consequences.**
- Walker cost is predictable and linear in commits, not in rename hops.
- Pure deletions credit the insertion-point symbol — approximation on purpose.
- Cost: a symbol moved across files loses attribution from pre-move commits. Agents investigating long-lived code history may see gaps. See [2_design.md §6.3] for the full caveat.

---

## ADR-007 — Task-ID attribution is a flat union, not state-aware

**Status:** Accepted (with open question) · 2026-04 · [T20260421-0528]

**Context.** A leaf touched by a reverted task and by a shipped task currently carries both IDs with no distinction. Consumers that want "which change is live" signal have to join against task status externally.

**Decision.** Keep `task_ids` a flat union at the graph layer. Do not embed lifecycle state in the graph.

**Consequences.**
- Graph stays independent of task-state evolution (which changes more often than code structure).
- Cost: consumers wanting shipped-only signal pay the external-join hit every query. [3_vision.md §1.11] may reopen this if the join proves too painful.

---

## ADR-008 — File-based lock store in `orbit-knowledge`, no standalone `orbit-lock` crate

**Status:** Accepted · 2026-04 · [T20260411-0424], [T20260417-0301-2]

**Context.** An earlier prototype factored graph-node locks into a standalone `orbit-lock` crate. The crate added a dependency edge without buying reuse — no consumer other than `orbit-knowledge` ever imported it.

**Decision.** Keep a file-based shared lock store inside `orbit-knowledge::lock`. Remove the `orbit-lock` crate.

**Consequences.**
- One fewer crate in the architecture diagram; simpler dependency graph.
- Locks remain file-backed and process-shareable, matching the content-addressed store's on-disk model.
- Cost: if a second consumer ever needs the same lock semantics, we'll re-extract — the shared-crate refactor would have prevented that future churn but would have paid for reuse we don't yet need. [T20260417-0301-2] closed holes around concurrent write paths.

---

## ADR-009 — Debounced, single-flighted refresh

**Status:** Accepted · 2026-04 · [T20260417-0307], [T20260416-0719], [T20260417-0639]

**Context.** Every read can trigger `ensure_fresh`. Without coordination, a dirty worktree plus many quick reads would stack rebuilds, and concurrent callers would duplicate work.

**Decision.** Guard rebuilds with a `flock` on `refresh.lock`. Debounce dirty-worktree rebuilds against a fingerprint + timestamp in `refresh_state.json` (default 5s, `ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS`). Concurrent callers wait briefly for the in-flight rebuild rather than starting their own.

**Consequences.**
- Steady-state read cost on a dirty worktree is one rebuild per debounce window, not one per read.
- Corrupt-store recovery path ([T20260416-0719]) lives in the same critical section.
- Cost: the first reader after a change pays full rebuild latency; subsequent readers ride the cache.

---

## ADR-010 — Orbit-owned symbol-level write operations

**Status:** Proposed · 2026-04 · [T20260421-0543]

**Context.** The identity matcher from [T20260421-0528] is the accepted limit of what `task_ids` attribution can recover from a refactor — it loses history on every rename, cross-file move, and split because its keys (`path`, `qualified_name`, `kind`) are exactly what those refactors change. The long-term shape aligned with the owner is that Orbit itself performs refactors through a symbol-level API and records the operation as a sidecar the rebuilder consumes; the read-side hook was already reserved in `pipeline/attribute.rs`.

**Decision.** Formalize the contract that hook validates against via [specs/graph-operations.md](./specs/graph-operations.md). Eight-op taxonomy (`create`, `delete`, `rename`, `move`, `change_signature`, `change_body`, `split`, `merge`), each atomic per entry — compound refactors emit multiple entries rather than a single "relocate" primitive. Address symbols by a graph-level **stable ID** (`stable_id: node:<nanoid-21>`) persisted on every node; rejected pre/post address pairs because disambiguation under simultaneous axis changes and N-ary ops (`split`/`merge`) requires a subject handle equivalent to a stable ID under a different name. Operations are **advisory-authoritative**: accepted when consistent with the commit's tree diff, ignored with a warning otherwise — the tree is always ground truth.

**Consequences.**
- `stable_id` preserves `task_ids` exactly across any refactor Orbit authors; the matcher remains the reconciliation layer for non-Orbit writes.
- The taxonomy is deliberately small — eight ops cover every refactor shape Orbit intends to own without requiring compound primitives.
- Divergence policy is asymmetric: ops can enrich attribution, never override. A producer bug cannot silently rewrite history; worst case is a fallback to the pre-producer behavior.
- Cost: `stable_id` is a new field on `BaseNodeFields` — schema bump on first rebuild after producer lands, and a one-time reallocation of object hashes for every existing node. Also: status stays `Proposed` until the producer ships; flip to `Accepted` + the producer's task ID at that time.

---

## ADR-011 — Non-code extraction via `FileKind`-dispatched extractors

**Status:** Accepted · 2026-04 · [T20260422-1540]

**Context.** Tree-sitter extractors covered five source-code languages; every other file landed in the graph as a leafless `FileNode`. Design docs under `docs/design/` and scoreboard/config files under `.orbit/` were load-bearing context but invisible to graph queries at sub-file granularity. The `LanguageExtractor` trait was the natural extension point — a pluggable design without plugins. Implementing a parallel system for non-code files would duplicate the registry and the pipeline dispatch.

**Decision.** Rename `LanguageExtractor` → `FileExtractor` and switch its discriminator from `Language` to a new `FileKind { Code(Language), Doc(DocFormat), Config(ConfigFormat), Table(TableFormat), Unknown }`. Add exactly three `LeafKind` variants: `Section { depth: u8 }` (markdown heading), `ConfigKey` (top-level key in YAML/JSON/TOML), `Column` (header cell in CSV/TSV). Ship shallow extractors only: ATX headings (not frontmatter, not fenced blocks), top-level map entries (not nested paths), first-row cells (not row-level nodes). 1 MiB size cap on tabular extraction short-circuits before parsing. Extraction is the only pipeline path that changes — `FileKind::from_extension` replaces `Language::from_extension` at build-time and attribute-time dispatch sites.

**Consequences.**
- Markdown section anchors, top-level config keys, and CSV columns are now first-class graph nodes with `task_ids` attribution flowing through the existing history walker (byte-range overlap is kind-agnostic).
- Stored index-file `kind` field switches from direct enum serialization to `LeafKind::to_string()` — required because `Section { depth }` serializes as `{"section": {"depth": 1}}` and the index's `kind: Option<String>` consumer expects bare strings. The full depth payload lives in the object body.
- Per-format map order for config keys is not part of the extractor contract — TOML is alphabetical, YAML/JSON are insertion-order. Consumers that care about order must sort.
- Cost: `LeafKind` JSON shape becomes heterogeneous (some variants are bare strings, `Section { depth }` is an externally-tagged object). Acceptable because no consumer pins the full LeafKind JSON string; `#[non_exhaustive]` not yet set on the enum — future LeafKind additions remain a breaking change for downstream exhaustive matches.

---

## ADR-012 — Keep scan-time graph inclusion separate from runtime policy access

**Status:** Accepted · 2026-04 · [T20260423-0452]

**Context.** The graph scanner originally filtered only through `git check-ignore`, which meant committed benchmark artifacts and other checked-in generated files still entered the graph and polluted search results. Reusing runtime policy for this would have mixed two different concerns: whether a path should be indexed at all versus whether an activity may read or modify it at runtime.

**Decision.** Introduce a scan-only `.orbitignore` layer in `orbit-knowledge`, implemented with the `ignore` crate and evaluated during `scan_repo` before parsing. Keep policy `denyRead` / `denyModify` in `orbit-policy` as a tool-call-time access control surface. Seed the default `.orbitignore` baseline into new workspaces during `orbit workspace init`, but preserve user-edited files once they exist.

**Consequences.**
- Index quality improves without coupling the scanner to runtime policy semantics or dependencies.
- Users get a visible, editable workspace-root file instead of hidden built-in behavior only.
- Git and Orbit ignore layers compose naturally: `.gitignore` handles Git-owned exclusions, `.orbitignore` handles committed-but-non-indexable paths.
- Cost: there are now two exclusion mechanisms that users can confuse, so the docs have to name the timing and intent boundary explicitly ([2_design.md §2.3]).

---

## ADR-013 — Changed-path incremental leaf reuse

**Status:** Accepted · 2026-04 · [T20260426-0140]

**Context.** Incremental rebuilds already computed `ctx.changed_paths`, but the leaf phase still re-read and re-extracted every extractable file. That made dirty-read refreshes O(repo) even when one file changed, and it wasted the content-addressed store's ability to preserve identical file/leaf objects.

**Decision.** During incremental builds, `build_graph_leaves` reads the previously persisted graph for the same branch ref and reuses unchanged file snapshots when the file source hash and every reused leaf's `file_hash_at_capture` match the new hash. Changed paths, new files, hash mismatches, absent refs, and unreadable prior graphs fall back to the normal extractor path; directory and file skeletons are still rebuilt from the current scan so deletes and ignore-rule changes are reflected.

**Consequences.**
- Single-file edits reduce extraction work from the whole repo to the changed path set.
- Zero-change incremental rebuilds can reproduce the previous root object hash byte-for-byte because reused leaves preserve IDs, identity keys, source hashes, and attribution fields.
- Deleted or newly ignored files naturally disappear because reuse only considers files in the current scan.
- Cost: extractor improvements do not automatically reparse unchanged files during an incremental rebuild; users need a full `orbit graph build` when extractor semantics, not file contents, are what changed.

---

## ADR-014 — Store-scoped LRU for graph objects and blobs

**Status:** Accepted · 2026-04 · [T20260426-0141]

**Context.** `KnowledgeStore` selector reads loaded content-addressed graph objects and source blobs through fresh per-call `HashMap`s. Repeated `pack`, `leaf_data`, and history queries against the same store therefore paid disk I/O, JSON parsing, and SHA-256 verification again for immutable hash-addressed data.

**Decision.** Add a `GraphObjectCache` owned by `KnowledgeStore`, backed by the `lru` crate with separate object and blob capacities. `read_graph_object` and `extract_leaf_source` consult that shared cache and run hash-integrity verification only on cache miss before insertion.

**Consequences.**
- Repeated selector reads on the same store avoid redundant object/blob filesystem reads and JSON parsing.
- Cache invalidation is content-hash based: changed nodes naturally use different keys, and old entries age out.
- Store-scoped ownership avoids cross-workspace bleed and keeps tests isolated.
- Cost: separate `KnowledgeStore` instances and separate CLI processes do not share entries; a future long-lived service cache may need a workspace-keyed layer if store instances are not retained.

---

## ADR-015 — Parallel per-file build work with ordered merge

**Status:** Accepted · 2026-04 · [T20260426-0139]

**Context.** Hashing and extractor dispatch were fully sequential even though each file can be read, hashed, and parsed independently. The graph writer is content-addressed, so any parallel implementation also had to preserve the previous file-order leaf stream and the unchanged-file reuse path from [T20260426-0140].

**Decision.** Add `rayon` to `orbit-knowledge` and parallelize only the per-file work. `compute_hashes` runs file reads and SHA-256 computation in workers, then replaces `ctx.new_hashes` after collecting the results. `build_graph_leaves` workers return per-file outputs or reusable prior snapshots; the main thread sorts by original `FileNode` index and mutates `ctx.graph.files` / `ctx.graph.leaves`.

**Consequences.**
- Full rebuilds can use available cores during the two most expensive file-local stages.
- `PipelineContext` remains single-owner mutable state, so the implementation avoids graph-level locks and shared mutation.
- Deterministic reassembly keeps `ctx.graph.leaves`, `FileNode.leaf_children`, and root object hashes stable relative to the sequential implementation.
- Cost: `orbit-knowledge` now has a direct `rayon` dependency, and future build-stage refactors must preserve ordered collection rather than pushing graph state from worker threads.

---

## ADR-016 — Task-id graph search as a scan filter, not a sidecar index

**Status:** Accepted · 2026-04 · [T20260426-0220]

**Context.** Nodes already carry `task_ids` from the attribution pass, but agents had no inverse lookup for "which selectors did this task touch?" A dedicated sidecar index could make that lookup faster, but it would add another persisted source of truth before usage patterns justify it.

**Decision.** Add `task_id` as an optional filter on `GraphContextService` search and expose it through `orbit.graph.search`. The filter exact-matches one task ID against each node's existing `task_ids` vector and composes with the existing query/type/kind/prefix/source-regex filters.

**Consequences.**
- Agents can answer review-prep and incident-inspection questions from the existing graph snapshot.
- No new graph schema, sidecar, or invalidation path is introduced.
- Cost: lookup remains O(nodes) and multi-task queries require repeated calls until a real usage pattern justifies an indexed or array-based surface.

---

## ADR-017 — Local graph build benchmark scoreboard over CI regression gates

**Status:** Accepted · 2026-04 · [T20260426-0236]

**Context.** Recent graph-build performance work proved wins with one-off manual benchmarks in task execution summaries. Those summaries are hard to compare after the task scrolls away, and the hot path (`ensure_fresh` before pack/search) can regress through pipeline, persistence, or cache changes. Criterion-style microbenchmarks would miss the dominant disk I/O and tree-sitter costs.

**Decision.** Add `make bench` as a local end-to-end graph build benchmark. The driver lives in `orbit-knowledge`, calls `pipeline::run_build` directly, runs a cold full build plus a warm incremental no-op build against the repo root by default, and appends wall time/RSS/count metrics to `.orbit/state/scoreboard/graph_bench.json` capped at 200 records.

**Consequences.**
- Developers can compare graph build trends with machine/core context and git SHA preserved beside the metrics.
- No CI gate is introduced; shared-runner noise would make absolute thresholds misleading.
- The default corpus is maintenance-free because it is the repo itself, but timings and counts move as the repo grows. Use the scoreboard for trend-watching, not cross-machine normalization.

---

## ADR-018 — Retain agent-facing `orbit_graph_*` MCP surface; provider-dependent value

**Status:** Accepted · 2026-04 · [T20260423-0524], [T20260426-0402]

**Context.** Three rounds of evidence asked whether the eight-tool agent-facing `orbit_graph_*` MCP surface earns its token cost against `grep` + `read`. Full record in [`5_null_result.md`](./5_null_result.md). v3 (the disposition round) gave codex first-class MCP parity with the same backend it had only reached through shell-exec in v1/v2. Codex hybrid utilization moved from 0/30 → 23/30 on the same model, with hybrid token cost 0.65× no-graph (aggregate) and graph-only the best codex arm on accuracy (30/30). Claude utilization stayed at 0/30 because its baseline tool surface already includes `Read` / `Grep` / `Glob`, so the graph tools competed against specialized fs primitives carrying higher base-rate familiarity rather than against a generic `exec_command`. Pre-registered cull thresholds: hybrid utilization ≥ 20% on at least one provider AND graph-only ≤ 1.3× no-graph per provider × fixture cell. Utilization passes (codex). Cost is mixed per-cell: codex passes 4 / 10 fixtures, claude 1 / 10.

**Decision.** Retain the agent-facing `orbit_graph_*` MCP surface. The retention rests primarily on criterion 1 (utilization), which codex carries decisively. The cost criterion is mixed even per-cell; the cells that pass include all four `locate` / `deps` / `trace` cases where graph is the structurally correct tool. This is a defensible product call, not a clean benchmark pass — see [`5_null_result.md`](./5_null_result.md) §"Decision" for the per-cell table and reasoning.

**Consequences.**
- The eight-tool `orbit_graph_*` MCP surface stays shipped.
- A diagnostic v4 round is planned to characterize the cost-overshoot fixtures, not to re-litigate keep/cull. Targets: the `impact-tool-context-struct-literals` firehose (12.43× on codex), the signature-vs-type-resolved precision gap, and payload-volume problems on `pack`-heavy navigations.
- Future work on schema-cache overhead and payload size (pointer-only graph reads, [T20260423-0607]) is a measured-need item, not speculative.
- Provider-dependent caveat: the surface earns its cost where the baseline tool list is generic (codex's `exec_command`-only). On providers whose baseline already includes specialized fs primitives that overlap in function (Claude), the data is consistent with "graph tools exist but don't get used" — a latent schema-cache tax paid without return. Whether eating that tax to keep codex happy is worth it is a product question, not a benchmark question.
- Future tool-surface decisions for other specialized orbit tooling should examine the same question: is the new tool competing in a shell selector (win), a tool-list selector against a generic alternative (win), or a tool-list selector against a specialized alternative (likely loss).

---

## ADR-019 — Keep the public graph surface read-only

**Status:** Accepted · 2026-04 · [T20260426-0453]

**Context.** Prototype graph mutation tools (`orbit.graph.add`, `orbit.graph.delete`, `orbit.graph.move`, and `orbit.graph.write`) implied that graph-node locks could coordinate write safety. That is not true for the workflow Orbit actually uses: agents commonly work in separate worktrees and branches, each with its own graph ref. A lock inside a branch-local graph snapshot cannot reliably serialize writes in another worktree.

**Decision.** Remove graph mutation tools from the public tool/MCP surface. Keep the current agent-facing graph API read-only: overview, search, show, pack, refs, callers, implementors, and deps. Use task `context_files` plus `orbit.task.locks.reserve` as the version's preflight write guard, with optimistic integration/review checks as the final authority for stale or overlapping edits. Internal working-graph and operation-log code may remain as deferred implementation substrate, but it is not advertised as a current agent API.

**Consequences.**
- Agents no longer see graph writes as a supported coordination mechanism.
- Write admission happens in a shared task/workflow plane rather than inside per-ref graph state, so it still has meaning before agents fan out into separate worktrees.
- Graph refs remain a read/index/context artifact, which matches their branch-scoped storage model.
- Cost: write guards are conservative at task context-file granularity, not precise at graph-node granularity. Fine-grained symbol-level mutation may return later only with a coordination story that works across worktrees.

---

## Task References

Tasks cited by ADRs above:

- **[T20260406-0455-3]** — Add Rust graph extractor.
- **[T20260407-0222]** — Refactor graph storage to content-addressed objects (origin of ADR-001).
- **[T20260409-0550]** — Validate and harden RustGraphExtractor.
- **[T20260409-0656]** — Leaf-level write tool with edit buffering and version chains.
- **[T20260411-0424]** — Consolidate `orbit-knowledge` crate; add tree-sitter extractors, build pipeline, lock store.
- **[T20260416-0236]** (+ `-2`, `-3`, `-4`) — WorkingGraph write-anchor validation, conflict/audit guarantees, canonical selectors, atomic moves.
- **[T20260416-0352]** — Add Go, Java, and JavaScript extraction support.
- **[T20260416-0719]** — Recover from corrupted knowledge graph store.
- **[T20260417-0301-2]** — Harden graph lock store.
- **[T20260417-0302]** — Durable source-file edits in working graph ops.
- **[T20260417-0307]** — Gate and guard graph refresh hot paths.
- **[T20260417-0639]** — Speed up workspace-init graph persistence.
- **[T20260421-0358]** — Branch-scoped refs.
- **[T20260421-0528]** — `task_ids` schema + git history walker.
- **[T20260421-0543]** — Orbit-owned symbol-level write operation schema ([specs/graph-operations.md](./specs/graph-operations.md)).
- **[T20260422-1540]** — Non-code extraction via `FileKind`-dispatched extractors (markdown, YAML/JSON/TOML, CSV/TSV).
- **[T20260423-0452]** — `.orbitignore` scan exclusions and separation from runtime policy access.
- **[T20260426-0139]** — Parallel per-file hashing and leaf extraction with ordered graph merge.
- **[T20260426-0140]** — Changed-path incremental leaf reuse.
- **[T20260426-0141]** — Store-scoped LRU for graph objects and blobs.
- **[T20260426-0220]** — Exact task-id filtering through `orbit.graph.search`.
- **[T20260426-0236]** — End-to-end graph build benchmark with scoreboard trend records.
- **[T20260423-0524]** — v3 graph MCP parity sweep (utilization & cost disposition).
- **[T20260423-0607]** — Pointer-only graph reads (deferred; cited by ADR-018 consequences).
- **[T20260426-0402]** — Land v3 retention decision in the ADR index.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface and standardize on task lock reservations as preflight write guards.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
