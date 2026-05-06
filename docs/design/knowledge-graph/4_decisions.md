# Knowledge Graph — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-06

ADR-style log of non-obvious knowledge-graph decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. The [T20260430-22] cleanup folds the former top-level evidence log into ADR-018 so this folder keeps only the convention-approved numbered docs.

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

**Status:** Accepted · 2026-04 · [T20260421-0358], [T20260505-1]

**Context.** The original layout used `.orbit/knowledge/graph/refs/current.json` — one mutable ref shared across every branch and worktree. The last rebuild won globally. Multi-branch and multi-worktree workflows therefore saw graph reads for the wrong branch, and concurrent rebuilds raced on the single pointer.

**Decision.** Namespace refs by branch: `refs/heads/<branch>.json`. Reads resolve the current git branch; writes fail on detached HEAD rather than invent a label. Reads fall back to the default branch's ref with a stderr warning when the current-branch ref does not yet exist; writes never fall back.

**Consequences.**
- Two worktrees on different branches can rebuild concurrently without corruption.
- A new branch remains readable via direct fallback, while auto-refresh materializes the current branch ref before treating the graph as fresh.
- Migration path: legacy `refs/current.json` is moved to `refs/heads/<default>.json` on open.
- Cost: two worktrees on the *same* branch still share a ref (see [2_design.md §6.5]).

---

## ADR-003 — Tree-sitter extractors over an LSP backend

**Status:** Accepted · 2026-04 (rollup updated 2026-05) · [T20260406-0455-3], [T20260409-0550], [T20260416-0352], [T20260505-11], [T20260505-13], [T20260505-14], [T20260505-15], [T20260505-16]

**Context.** Reference resolution is strongest via a language server, but LSPs are stateful long-running processes tuned for interactive UX. Agent tools want bulk, structured, token-budgeted output and low lifecycle overhead.

**Decision.** Use tree-sitter grammars with per-language extractors producing structural symbols only. Defer cross-file reference resolution indefinitely (see [3_vision.md §1.1] for the open question of re-introducing LSP as a pluggable backend). Each new language extends this decision via the table below rather than a new ADR; only language-specific tradeoffs that would surprise a reader (a new `LeafKind` variant, an excluded extension, a non-obvious mapping) earn a row in the Notes column.

| Language | Extensions | Grammar | Task(s) | Notes |
|----------|------------|---------|---------|-------|
| Rust | `.rs` | `tree-sitter-rust` | [T20260406-0455-3], [T20260409-0550] | — |
| Go, Java, JavaScript | `.go`, `.java`, `.js` | upstream | [T20260416-0352] | — |
| Python | `.py` | upstream | (existing) | — |
| TypeScript / TSX | `.ts`, `.mts`, `.cts`, `.tsx`, `.d.ts` | `tree-sitter-typescript` | [T20260505-11] | Adds `enum` and `type_alias` LeafKinds. `.d.ts` classifies through its `.ts` extension. |
| C# | `.cs` | `tree-sitter-c-sharp` | [T20260505-13] | `.csx`, `.cshtml`, and Razor-style files explicitly excluded until separate extractors land. |
| Kotlin | `.kt`, `.kts` | `tree-sitter-kotlin-ng` | [T20260505-14] | Adds `package`, `object`, `companion_object` kinds. Extension functions emit as standalone `function` leaves named `Receiver.function`. |
| Ruby | `.rb` | upstream | [T20260505-15] | — |
| C | `.c`, `.h` | `tree-sitter-c` | [T20260505-16] | Headers share the C extractor; prototypes emit as `function_declaration`. C++-shaped headers stay C-classified until a separate extractor lands. |

**Consequences.**
- Fast, deterministic extraction with no per-query process lifecycle.
- `find_references`, `callers`, `implementors` ([T20260412-0645-3]) are signature-matched, not type-resolved — a superset of the truth.
- Adding a language is an instance of this ADR, not a new decision: append a row above and cite the task on the Status line.
- Cost: extractor maintenance scales with languages supported; overloads and partial declarations still share syntax-level names until a future signature-aware identity scheme lands.
- Cost: the graph `LeafKind` surface grows as languages add their own kinds (`enum`, `type_alias`, `package`, `object`, `companion_object`, `function_declaration`); downstream exhaustive matches must absorb each addition.

---

## ADR-004 — Shell out to the `git` CLI instead of an in-process library

**Status:** Accepted · 2026-04 · [T20260421-0528]

**Superseded by:** ADR-029 / [T20260506-11].

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

**Context.** `git log --follow` chases renames through history but at non-trivial per-hop cost. Hunk coordinates have to be re-mapped after every rename hop. At commit volumes typical of this repo, follow mode compounds into minutes of extra walker time. This decision described the now-removed attribution walker.

**Decision.** Map hunks to leaves by line-range overlap against the symbol's span *at the commit's tree*. Do not chase renames. A symbol moved across files gets attribution from post-move commits only.

**Consequences.**
- Walker cost is predictable and linear in commits, not in rename hops.
- Pure deletions credit the insertion-point symbol — approximation on purpose.
- Cost: a symbol moved across files loses attribution from pre-move commits. Agents investigating long-lived code history may see gaps. See [2_design.md §6.3] for the full caveat.

---

## ADR-007 — Task-ID attribution is a flat union, not state-aware

**Status:** Accepted (with open question) · 2026-04 · [T20260421-0528]

**Superseded by:** ADR-029 / [T20260506-11].

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

**Status:** Accepted · 2026-04 · [T20260417-0307], [T20260416-0719], [T20260417-0639], [T20260505-1]

**Context.** Every read can trigger `ensure_fresh`. Without coordination, a dirty worktree plus many quick reads would stack rebuilds, and concurrent callers would duplicate work.

**Decision.** Guard rebuilds with a `flock` on `refresh.lock`. Debounce dirty-worktree rebuilds against a fingerprint + timestamp in `refresh_state.json` (default 5s, `ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS`). Freshness also requires the current branch ref to exist, so debounce cannot suppress the first build for a missing branch ref. Concurrent callers wait briefly for the in-flight rebuild rather than starting their own.

**Consequences.**
- Steady-state read cost on a dirty worktree is one rebuild per debounce window, not one per read.
- Corrupt-store recovery path ([T20260416-0719]) lives in the same critical section.
- Cost: the first reader after a change pays full rebuild latency; subsequent readers ride the cache.

---

## ADR-010 — Orbit-owned symbol-level write operations

**Status:** Proposed · 2026-04 · [T20260421-0543]
**Superseded by:** [T20260506-11] for graph task-attribution preservation.

**Context.** This ADR was written when `task_ids` attribution was still a graph feature. That feature was removed in ADR-029 / [T20260506-11], so the attribution-preservation motivation is historical. The remaining symbol-operation taxonomy may still inform future graph write tools, but it no longer has an attribution consumer.

**Decision.** Formalize the contract that hook validates against via [specs/graph-operations.md](./specs/graph-operations.md). Eight-op taxonomy (`create`, `delete`, `rename`, `move`, `change_signature`, `change_body`, `split`, `merge`), each atomic per entry — compound refactors emit multiple entries rather than a single "relocate" primitive. Address symbols by a graph-level **stable ID** (`stable_id: node:<nanoid-21>`) persisted on every node; rejected pre/post address pairs because disambiguation under simultaneous axis changes and N-ary ops (`split`/`merge`) requires a subject handle equivalent to a stable ID under a different name. Operations are **advisory-authoritative**: accepted when consistent with the commit's tree diff, ignored with a warning otherwise — the tree is always ground truth.

**Consequences.**
- The taxonomy is deliberately small — eight ops cover every refactor shape Orbit intends to own without requiring compound primitives.
- Any future producer needs a new consumer and migration story; attribution preservation is no longer enough rationale.
- Cost: `stable_id` would still be a new field on `BaseNodeFields` if revived — schema bump on first rebuild after producer lands, and a one-time reallocation of object hashes for every existing node. Also: status stays `Proposed` until the producer ships; flip to `Accepted` + the producer's task ID at that time.

---

## ADR-011 — Non-code extraction via `FileKind`-dispatched extractors

**Status:** Accepted · 2026-04 · [T20260422-1540]

**Context.** Tree-sitter extractors covered five source-code languages; every other file landed in the graph as a leafless `FileNode`. Design docs under `docs/design/` and scoreboard/config files under `.orbit/` were load-bearing context but invisible to graph queries at sub-file granularity. The `LanguageExtractor` trait was the natural extension point — a pluggable design without plugins. Implementing a parallel system for non-code files would duplicate the registry and the pipeline dispatch.

**Decision.** Rename `LanguageExtractor` → `FileExtractor` and switch its discriminator from `Language` to a new `FileKind { Code(Language), Doc(DocFormat), Config(ConfigFormat), Table(TableFormat), Unknown }`. Add exactly three `LeafKind` variants: `Section { depth: u8 }` (markdown heading), `ConfigKey` (top-level key in YAML/JSON/TOML), `Column` (header cell in CSV/TSV). Ship shallow extractors only: ATX headings (not frontmatter, not fenced blocks), top-level map entries (not nested paths), first-row cells (not row-level nodes). 1 MiB size cap on tabular extraction short-circuits before parsing. Extraction is the only pipeline path that changes — `FileKind::from_extension` replaces `Language::from_extension` at build time.

**Consequences.**
- Markdown section anchors, top-level config keys, and CSV columns are now first-class graph nodes.
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
- Zero-change incremental rebuilds can reproduce the previous root object hash byte-for-byte because reused leaves preserve IDs, identity keys, and source hashes.
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

**Superseded by:** ADR-029 / [T20260506-11].

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

**Context.** Three benchmark rounds asked whether the eight-tool agent-facing `orbit_graph_*` MCP surface earns its token cost against grep/read. v1/v2 first looked like a null result, but codex only had shell access to graph tools there; v3 gave codex MCP parity and hybrid utilization moved from 0/30 to 23/30, with hybrid aggregate tokens at 0.65× no-graph and graph-only accuracy at 30/30. Claude stayed at 0/30 hybrid graph use because its baseline already included specialized `Read` / `Grep` / `Glob`. The pre-registered keep threshold passed on utilization but remained mixed on per-cell cost: codex passed 4/10 fixtures, claude 1/10.

**Decision.** Retain the agent-facing `orbit_graph_*` MCP surface. The decisive signal is provider-specific utilization when the graph tools are first-class; the cost signal is useful but not a clean pass. The duplicated top-level evidence log was folded into this ADR and removed under [T20260430-22].

**Consequences.**
- The eight-tool `orbit_graph_*` MCP surface stays shipped.
- A diagnostic v4 round is planned to characterize the cost-overshoot fixtures, not to re-litigate keep/cull. Targets: the `impact-tool-context-struct-literals` firehose (12.43× on codex), the signature-vs-type-resolved precision gap, and payload-volume problems on `pack`-heavy navigations.
- Future work on schema-cache overhead and payload size (pointer-only graph reads, [T20260423-0607]) is a measured-need item, not speculative.
- Provider-dependent caveat: the surface earns its cost where the baseline tool list is generic (codex's `exec_command`-only). On providers whose baseline already includes specialized fs primitives that overlap in function (Claude), the data is consistent with "graph tools exist but don't get used" — a latent schema-cache tax paid without return. Whether eating that tax to keep codex happy is worth it is a product question, not a benchmark question.
- Future tool-surface decisions for other specialized orbit tooling should examine the same question: is the new tool competing in a shell selector (win), a tool-list selector against a generic alternative (win), or a tool-list selector against a specialized alternative (likely loss).
- Future benchmark thresholds must specify both a per-cell threshold and the aggregation rule before a sweep runs.
- Cost: the MCP schema payload and prompt budget tax remain provider-dependent; retaining the surface deliberately accepts that overhead for providers that do use it.

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

## ADR-020 — Configurable task-ID extraction with manifest-driven backfill

**Status:** Accepted · 2026-04 · [T20260426-0507]

**Superseded by:** ADR-029 / [T20260506-11].

**Context.** Task-ID attribution was hardcoded to the Orbit format `\[T\d{8}-\d{4}(?:-\d+)?\]` in two places (`pipeline/history.rs` and `service/history.rs`). Codebases using Jira (`PROJ-123`), Linear (`ENG-123`), or GitHub-issue (`#123`) conventions saw empty graph-backed results and a silently empty fallback — the feature was unusable outside Orbit's own repo. The same `orbit task history` CLI surface also lived under the wrong subcommand: it never touches task lifecycle, only the graph.

**Decision.** Move the CLI to `orbit graph history <selector>` (drop the redundant `rebuild` subsubcommand; `orbit graph build` does the same job). Introduce a single `TaskIdPattern` accessor in `orbit-knowledge` consumed by both the build-time attribution pass and the history fallback. Expose configuration through `--task-id-pattern <regex>` on `orbit graph build` / `orbit graph history` and `knowledge.task_id_pattern` in workspace `config.toml`, with strict precedence CLI flag > config > Orbit default. Adopt a capture-group convention (group 1 if present, else whole match) so the default Orbit pattern strips brackets in-regex instead of bespoke string slicing — the stored task IDs stay byte-identical to pre-T20260426-0507 graphs. Persist the pattern in `manifest.json`. When the configured pattern differs from the manifest pattern, `orbit graph history` emits a stderr warning, and the attribution pass forces a full-history backfill (cursor reset, prior task_ids hydration skipped) so the new pattern repopulates every node. Also add `orbit.graph.history` as an MCP/agent tool returning the same JSON shape as the CLI's `--json`.

**Consequences.**
- Non-Orbit codebases can now use the feature without forking; orbit's own repo gets identical output to before.
- A pattern change is safe: a subsequent `orbit graph build` is guaranteed to backfill correctly rather than silently leave stale `task_ids` from the prior pattern.
- The agent tool surface stays in sync with the CLI: a future schema change must update both.
- Cost: a pattern change incurs a full-history walk on next build. This is the right default — silently skipping history would be the worse failure mode.
- Cost: validating regex twice (orbit-core config + orbit-knowledge consumer) — accepted to keep `orbit-core` free of the `orbit-knowledge` dep.

---

## ADR-021 — Route graph CLI through the tools facade

**Status:** Accepted · 2026-04 · [T20260426-2042]

**Context.** `orbit-cli` imported `orbit-knowledge` directly for graph build/update, show/search, history payloads, workspace-init graph build, and `.orbitignore` defaults. That made clap command files a second graph application layer and duplicated JSON shaping already present in the agent tool surface.

**Decision.** Move those graph use cases into `orbit-tools::graph`, re-export them from `orbit-core::command::graph`, and keep `orbit-cli` on clap parsing plus human output. `orbit-core` continues to avoid a direct `orbit-knowledge` dependency; `orbit-tools` remains the only upstream graph consumer.

**Consequences.**
- `orbit-cli` no longer declares or imports `orbit-knowledge`.
- CLI and agent graph surfaces share the JSON payload builders for `show` and `history`.
- Workspace init still seeds `.orbitignore` and attempts the initial graph build through explicit core helpers.
- Cost: `orbit-tools` now contains a user-facing graph facade in addition to registered tools, so future maintainers must distinguish reusable use-case helpers from tool schemas and avoid accidentally registering CLI-only build/update behavior as agent tools.

---

## ADR-022 — Align graph task-ID grammar with task-store IDs

**Status:** Accepted · 2026-04 · [T20260428-1]

**Superseded by:** ADR-029 / [T20260506-11].

**Context.** `orbit.graph.search` added an exact `task_id` filter in [T20260426-0220], but its input validator hardcoded `T\d{8}-\d{4}`. The task store now creates unpadded daily suffixes such as `T20260428-1`, while historical graph/task references also include amended numeric suffixes such as `T20260412-0645-2`. The graph attribution default still only matched the older four-digit base suffix, so a selector-first task lookup could fail before search, or miss current task IDs after a rebuild.

**Decision.** Treat the bare Orbit task-ID body accepted by graph attribution/search as `T\d{8}-\d+(?:-\d+)*`. Keep the configurable `TaskIdPattern` mechanism from ADR-020; this change only updates Orbit's default pattern and the agent-facing `orbit.graph.search` input validator.

**Consequences.**
- Current task-store IDs, historical four-digit IDs, and amended numeric IDs all share one graph default.
- A workspace with a manifest written under the older default will see the existing manifest-pattern mismatch path and get a full-history backfill on the next graph build.
- Cost: the default is intentionally more permissive about leading zeros and amendment depth so existing historical IDs stay queryable; task creation remains governed by the task store.

---

## ADR-023 — Keep MCP graph exposure equal to the read-only graph tool set

**Status:** Accepted · 2026-04 · [T20260428-3]

**Context.** The MCP safe list exposed the original eight read-only graph tools, but `orbit.graph.history` later joined the registered graph tool surface as a read-only history/query tool. That left Codex and Gemini MCP discovery dependent on a stale client-visible safe list even though the runtime registry had the tool.

**Decision.** Treat the MCP graph surface as the full read-only graph tool set: callers, deps, history, implementors, overview, pack, refs, search, and show. Continue excluding graph mutation tools (`add`, `delete`, `move`, `write`) from the public MCP surface.

**Consequences.**
- Codex, Gemini, and other MCP clients discover the same read-only graph capabilities that `orbit tool list` reports as active.
- The history tool is now a compatibility stub after ADR-029; agents should use `git log --grep '[T<task-id>]'` for local forward lookup.
- Cost: the MCP schema payload gains one more graph tool, so the provider-dependent schema-cache caveat from ADR-018 still applies.

---

## ADR-024 — TypeScript and TSX use dedicated tree-sitter grammars

**Status:** Superseded by ADR-003 (folded 2026-05) · 2026-05 · [T20260505-11]

Folded into ADR-003's per-language coverage table. See that entry for the current shape.

---

## ADR-025 — Pack favors prompt-time responsiveness over inline refresh

**Status:** Accepted · 2026-05 · [T20260505-5]
**Author:** gpt-5.5

**Context.** Agents use `orbit.graph.pack` at the start of execution to turn task context selectors into prompt material. Letting that call trigger an unbounded inline graph refresh can make the selector-first workflow appear hung, with no partial selector results or timeout hint.

**Decision.** Make `orbit.graph.pack` read the existing graph snapshot by default and return an `auto_refresh.skipped` diagnostic that names the explicit refresh path. Add a `refresh: true` opt-in for callers that accept a potentially slow inline refresh, and add `timeout_ms` so selector projection can return unresolved entries for selectors not reached before the budget expires.

**Consequences.**
- Context-gathering agents get prompt-visible guidance instead of a silent rebuild when the snapshot is stale.
- Timed-out pack calls can still return the selectors already projected plus unresolved entries for the remainder.
- Cost: default pack reads can be stale until a separate `orbit graph build` or an opt-in refresh updates the branch ref.

## ADR-026 — C source and headers share one extractor

**Status:** Superseded by ADR-003 (folded 2026-05) · 2026-05 · [T20260505-16]

Folded into ADR-003's per-language coverage table. See that entry for the current shape.

---

## ADR-027 — Kotlin mirrors Java-style tree-sitter extraction

**Status:** Superseded by ADR-003 (folded 2026-05) · 2026-05 · [T20260505-14]

Folded into ADR-003's per-language coverage table. See that entry for the current shape.

---

## ADR-028 — C# uses syntax-only enterprise coverage

**Status:** Superseded by ADR-003 (folded 2026-05) · 2026-05 · [T20260505-13]

Folded into ADR-003's per-language coverage table. See that entry for the current shape.

---

## ADR-029 — Remove graph task attribution

**Status:** Accepted · 2026-05 · [T20260506-11]
**Author:** gpt-5

**Context.** Orbit carried a task attribution pipeline that parsed task IDs from commit messages, mapped hunks back to graph nodes, persisted `task_ids`/`structural_conflict` fields, wrote a task-commits sidecar, and exposed reverse lookup through `orbit.graph.search task_id` and `orbit.graph.history`. A 10-day audit window from 2026-04-26 to 2026-05-06 found 961 `orbit.graph.*` tool calls and 0 uses of the reverse-lookup parameters. The forward lookup users actually need is already native git text search: `git log --grep '[T<task-id>]'`. Separately, the task-sync doctrine now treats Orbit `task_id` as local-only; cross-engineer references go through `external_refs`.

**Decision.** Remove graph task attribution. Delete the attribution pipeline, `TaskIdPattern`, task-id search/history parameters, node attribution fields, sidecar persistence, and manifest/config plumbing. Keep `orbit.graph.history` as a compatibility stub that returns a clear removal message and points to `git log --grep`. Preserve commit-message `[T...]` convention as a local search key.

**Consequences.**
- Graph build no longer pays attribution-walker cost or persists unread reverse-lookup data.
- Legacy graph objects and refs that still contain attribution fields load through serde unknown-field tolerance; the fields disappear on rebuild.
- `knowledge.task_id_pattern` is deprecated and ignored with a one-line warning instead of failing old configs.
- Cross-engineer task references are explicit through `external_refs`, not inferred from local Orbit task IDs.
- Cost: users who depended on reverse lookup from selector to tasks lose that graph query. The documented replacement only covers forward lookup from task ID to commits.

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
- **[T20260421-0528]** — Historical `task_ids` schema + git history walker; removed by ADR-029 / [T20260506-11].
- **[T20260421-0543]** — Orbit-owned symbol-level write operation schema ([specs/graph-operations.md](./specs/graph-operations.md)).
- **[T20260422-1540]** — Non-code extraction via `FileKind`-dispatched extractors (markdown, YAML/JSON/TOML, CSV/TSV).
- **[T20260423-0452]** — `.orbitignore` scan exclusions and separation from runtime policy access.
- **[T20260426-0139]** — Parallel per-file hashing and leaf extraction with ordered graph merge.
- **[T20260426-0140]** — Changed-path incremental leaf reuse.
- **[T20260426-0141]** — Store-scoped LRU for graph objects and blobs.
- **[T20260426-0220]** — Historical exact task-id filtering through `orbit.graph.search`; removed by ADR-029 / [T20260506-11].
- **[T20260426-0236]** — End-to-end graph build benchmark with scoreboard trend records.
- **[T20260423-0524]** — v3 graph MCP parity sweep (utilization & cost disposition).
- **[T20260423-0607]** — Pointer-only graph reads (deferred; cited by ADR-018 consequences).
- **[T20260426-0402]** — Land v3 retention decision in the ADR index.
- **[T20260426-0453]** — Remove graph write operations from the public tool/MCP surface and standardize on task lock reservations as preflight write guards.
- **[T20260426-0507]** — Historical `orbit graph history` and configurable task-ID regex; attribution behavior removed by ADR-029 / [T20260506-11].
- **[T20260426-2042]** — Move graph CLI behavior behind the `orbit-tools::graph` facade and remove the direct `orbit-knowledge` dependency from `orbit-cli`.
- **[T20260428-1]** — Historical graph task-ID attribution/search alignment; removed by ADR-029 / [T20260506-11].
- **[T20260428-3]** — Expose the full read-only graph tool set through the MCP safe surface for Codex and Gemini.
- **[T20260430-22]** — Compact the knowledge-graph design docs and fold the obsolete evidence log into ADR-018.
- **[T20260505-1]** — Require auto-refresh freshness checks to materialize missing current-branch graph refs before returning fresh.
- **[T20260505-5]** — Bound `orbit.graph.pack` selector gathering and skip inline refresh by default.
- **[T20260505-11]** — Add TypeScript and TSX classification, extraction, and graph search/pack coverage.
- **[T20260505-13]** — Add C# classification, tree-sitter extraction, and graph search coverage for .NET workspaces.
- **[T20260505-14]** — Add Kotlin classification, tree-sitter extraction, and graph search coverage for mixed Java/Kotlin workspaces.
- **[T20260505-15]** — Add Ruby classification, tree-sitter extraction, graph search coverage, and Ruby symbol kinds.
- **[T20260505-16]** — Add C and header classification, tree-sitter extraction, and graph search coverage.
- **[T20260506-11]** — Remove graph task attribution after audited reverse-lookup usage was 0/961; preserve `[T...]` as a local commit-search key.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
