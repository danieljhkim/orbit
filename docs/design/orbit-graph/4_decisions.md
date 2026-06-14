---
summary: "Orbit Graph — Decisions"
type: design
title: "Orbit Graph — Decisions"
owner: claude
last_updated: 2026-06-13
status: Draft
feature: orbit-graph
doc_role: decisions
tags: ["orbit-graph"]
related_features: [knowledge-graph]
---

# Orbit Graph — Decisions

ADR-style log of non-obvious `orbit-graph` decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by global ADR ID; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. Cost lines are mandatory.

---

## ADR-0184 — Graph is a derived index, not a versioned store

**Status:** Superseded by ADR-0195 · 2026-06-13 · [ORB-00294] [ORB-00377]

**Context.** `orbit-knowledge` was built as a git-like history layer: content-addressed objects, mutable refs, atomic swaps, lock protocols. In practice the graph is consumed as "fresh queryable index of the current code" — none of the version-store affordances are used by agents.

**Decision.** Reframe the graph as a derived index, regenerable from `(file_contents, extractor_version)`. Delete object storage, mutable refs, and atomic-swap locking. Single SQLite file per worktree is the only durable state.

**Consequences.**
- Deletes ~3k LOC of object-store, lock, and ref-management code.
- Removes the lock protocol's structural inability to coordinate same-branch worktrees (see knowledge-graph ADR-002 cost line).
- Cost: **no history.** "What did the graph look like at commit X?" is no longer a query the graph can answer. Use git for that.

## ADR-0185 — Per-worktree DB filename embeds extractor version

**Status:** Proposed · 2026-05 · [ORB-00294]

**Context.** When extractor logic changes (new language, fixed parse bug, schema tweak), the on-disk DB becomes incompatible. The traditional fix is schema migration code; the V1 ethos is to keep complexity out of the storage layer.

**Decision.** DB filename is `<branch>.<extractor_version>.db`. Bumping `EXTRACTOR_VERSION` makes old DBs invisible; they're deleted on next sync. No migration code.

**Consequences.**
- Extractor version bumps are zero-friction; agents never see migration failures.
- Multiple extractor versions can coexist on disk temporarily during rollback testing.
- Cost: **cold rebuild after every extractor bump.** For a 200k LOC repo that's ~3s, acceptable per the perf budget. For a much larger repo it could become noticeable; revisit if a user complains.

## ADR-0186 — Symbol IDs are ephemeral; resolution by qualified name

**Status:** Proposed · 2026-05 · [ORB-00294]

**Context.** With incremental sync, a file's symbol rows are deleted and re-inserted on change. If cross-file refs FK to `symbols.id`, every incremental rebuild orphans inbound refs from other files. The current `orbit-knowledge` schema has an `identity_key` column trying to paper over this; it doesn't fully work and adds complexity.

**Decision.** No foreign key on `symbols.id` from any table. Refs and relations resolve by `target_qualified` (string lookup). A `target_symbol_hint INTEGER` column exists as a build-time cache but is non-authoritative.

**Consequences.**
- Incremental sync is correct by construction: dropping a file's symbols doesn't dangle anything.
- No `identity_key` column or cross-build lineage tracking machinery.
- Cost: **string lookups instead of integer FK joins.** SQLite's B-tree on `target_qualified` keeps this fast (low single-ms even on 100k symbols), but it's a real cost compared to the natural FK design. Rename tracking is a separate feature on top of git, not a graph affordance.

## ADR-0187 — Two ref tables, not one

**Status:** Proposed · 2026-05 · [ORB-00294]

**Context.** The original draft put all cross-symbol edges in one `refs` table with a `kind` column covering `call | type | impl | use | trait_bound`. Calls and type uses are anchored to `(file, span)`. Impl relations are anchored to `(concrete_symbol, trait_symbol)` with no useful span. Mixing them forces meaningless columns on the impl side.

**Decision.** Split into `refs` (textual, `from_file + from_span_start/end`) and `relations` (symbol-to-symbol, `from_qualified + to_qualified`). CLI `--kind impl` is a routing alias to `relations`.

**Consequences.**
- "What implements X?" is a single `relations` index lookup, fast enough to be a hot path.
- The two tables are independently extensible (e.g. adding `relations.kind = "annotates"` for TypeScript decorators) without inflating the `refs` shape.
- Cost: **two indexes to maintain instead of one.** Schema is wider; the `refs` command needs to union two underlying queries. Acceptable for the correctness and ergonomics gain.

## ADR-0188 — Sync policy is a property of the Graph handle

**Status:** Proposed · 2026-05 · [ORB-00294]

**Context.** The original draft hardcoded "10ms stat budget at 5000 files; cache window 500ms" inside the query layer. The budget doesn't scale, and the policy mixes product decisions into the library.

**Decision.** `Graph::open(root, policy: SyncPolicy)` where `SyncPolicy` is `Manual | OnRead | Windowed { window: Duration }`. CLI default: `Manual`. MCP server default: `Windowed { window: 500ms }`.

**Consequences.**
- Tests use `Manual` for determinism; long-lived processes use `Windowed`; one-shot scripts can use `OnRead` for paranoia.
- The library no longer carries an implicit perf contract that breaks silently at scale.
- Cost: **callers must choose.** No "just works" default beyond per-entry-point conventions. The conventions are documented but the choice is exposed.

## ADR-0189 — Performance gate is against committed baseline, not last run

**Status:** Proposed · 2026-05 · [ORB-00294]

**Context.** A perf regression gate that compares "this run vs previous merged run" ratchets up to whatever the latest measurement happened to be — slow degradation goes undetected.

**Decision.** Baseline lives at `bench/baselines.json`, committed to the repo. Regression gate fires when a run is >20% slower than the *committed* baseline. Bumping the baseline requires a labeled PR and a one-line justification.

**Consequences.**
- Slow erosion is caught; cumulative drift requires an explicit acknowledgment.
- Performance wins are realized by intentional baseline bumps, not silent improvements that immediately become the new floor.
- Cost: **baseline updates are friction.** Every routine improvement requires a labeled PR. Acceptable — the friction is intentional and the alternative (no friction, no guarantee) is worse.

## ADR-0190 — Use per-commit DB files for detached HEAD

**Status:** Accepted · 2026-05-25 · [ORB-00331]

**Context.** ORB-00326 (78e26efa) fixed detached-HEAD meta recording, but the filename still used `HEAD.<version>.db`, so detached checkouts on different commits churned the same cache. ORB-00331 compared keeping one `HEAD` DB with warnings against giving each detached commit its own DB file.

**Decision.** Detached HEAD uses `detached-<short-sha>.<extractor_version>.db`. Branch-attached checkouts keep the existing `<branch>.<extractor_version>.db` layout, and detached meta still records `branch = "HEAD"` plus the full commit SHA.

**Consequences.**
- Detached checkouts on different commits no longer invalidate each other through the same `HEAD` database.
- The stale-DB sweep removes detached DBs whose commits are no longer reachable from any local ref, while preserving the active DB family.
- Cost: **more files during commit-hopping workflows.** Bisects and cherry-picks can create O(N) detached DBs until reachability cleanup prunes the unreachable ones.

## ADR-0192 — Roll back orbit-graph tool cutover to orbit-knowledge

**Status:** Accepted · 2026-05-25 · [ORB-00344] · Supersedes ADR-0191 · Amended by ADR-0197 (equivalence-harness retention reversed)

**Context.** ORB-00338 cut the active graph query tools over from `orbit-knowledge` to `orbit-graph`, but audit data and post-cutover testing found unacceptable steady-state regressions: 13.5x p50 search slowdown, a roughly 9s cold-call floor, deleted high-use tools, incomplete plugin MCP exposure, byte-array `show` output, empty `trace` results for real enum-dispatch commands, and direction-confused `impact` output.

**Decision.** Restore the legacy `orbit-knowledge`-backed `orbit.graph.search`, `show`, `refs`, `callers`, `pack`, `overview`, `implementors`, and `deps` surface as the active backend. Keep the `orbit-graph` crate and equivalence harness in tree, but gate any future cutover on the rollback learnings captured in the global ADR.

**Consequences.**
- Future cutover work must use `SyncPolicy::Manual` as the query-tool default unless a measured long-lived process explicitly opts into another policy.
- Pre-cutover audit-log analysis, plugin MCP exposure equivalence, UTF-8 text response boundaries, trace/impact correctness gates, and cold-call latency measurements are required before another backend swap.
- Lost for now: cutover-only `callees`, `impact`, `trace`, the changed `sync` shape, and the extended graph-equiv corpus.
- Cost: **cutover pauses.** The `orbit-graph` backend remains available for development, but agents lose the new cutover-only APIs until the root causes are fixed and a new cutover passes the gates.

## ADR-0195 — Watcher-backed graph reads

**Status:** Accepted · 2026-06-13 · [ORB-00377] · Supersedes ADR-0188

**Context.** ORB-00377 found that the MCP `orbit.graph.*` read path was effectively poll-on-read: the 500ms `Windowed` policy elapsed between most agent calls, so each query paid for a full worktree diff before running the SQLite lookup. Lengthening the window would reduce frequency but would keep query latency coupled to repository size.

**Decision.** Long-lived MCP graph handles use a watcher-backed policy: `Graph::open` performs one initial auto sync, starts a `notify` watcher scoped to the worktree, coalesces relevant filesystem events behind a debounce, and runs sync in the background. Query methods do not run inline sync for this policy; they read from a cached SQLite connection. The freshness contract is eventual: after a same-process file edit, graph reads may remain stale until the watcher observes and syncs the event, normally within the debounce plus sync duration; callers needing a hard read-after-write barrier must call `Graph::sync`/`orbit.graph.sync` before querying.

**Consequences.**
- Repeated graph reads with no intervening edits are pure SQLite lookups and do not initiate scanner walks.
- Watcher overflow or watcher errors request a coalesced auto sync, preserving the conservative fallback path.
- `Windowed` remains available as an explicit fallback policy, but it is no longer the MCP default.
- Cost: the MCP process now depends on platform filesystem watcher behavior and may serve stale graph data during the documented debounce-plus-sync window.

## ADR-0197 — Remove the orbit-graph equivalence and benchmark harness

**Status:** Accepted · 2026-06-13 · [ORB-00385] · Amends ADR-0192

**Context.** ADR-0192 kept the `orbit-graph` crate and the equivalence harness in tree to gate a future v2 cutover. The harness — the `tools/graph-equiv` workspace binary (plus its frozen corpus) and the `bench/` baseline scripts — dual-ran both backends over a frozen corpus and failed CI (`make ci-equiv`, the `graph-equiv` CI job) on diffs outside documented tolerances. With the cutover paused indefinitely and none scheduled, that gate carried standing cost (a workspace member, a CI job, a Makefile target, a dependency-direction guardrail entry) for an inactive migration step.

**Decision.** Remove `tools/graph-equiv/` and `bench/` and unwire them from the build — the Cargo workspace member, the `make ci-equiv` target, the `graph-equiv` CI job, and the `check-dependency-direction.sh` allowlist entry. This amends ADR-0192's "keep the equivalence harness in tree" consequence *only*; ADR-0192's rollback decision and pre-cutover gates remain in force, and the `orbit-graph` crate is kept. The equivalence relation in [`GRAPH_SPEC.md`](specs/GRAPH_SPEC.md) still defines what a future cutover must satisfy; the harness is reintroduced fresh when a cutover is rescheduled rather than carried as an inactive scaffold.

**Consequences.**
- The workspace drops one crate and the `graph-equiv` CI job; `make ci-equiv` no longer exists.
- The documented v1↔v2 equivalence relation is now plan-only — no in-tree binary enforces it until a cutover is rescheduled.
- ADR-0192's harness-retention consequence is amended here; its rollback decision is unchanged.
- Cost: a future cutover must rebuild the equivalence + benchmark harness (binary, frozen corpus, baselines, CI wiring) from scratch; the existing corpus and baseline history are lost.

---

## Task References

- [ORB-00294] allocated the six initial orbit-graph ADR IDs (ADR-0184 through ADR-0189).
- [ORB-00331] allocated ADR-0190 and shipped the detached-HEAD per-commit DB layout.
- [ORB-00344] allocated ADR-0192 and restored `orbit-knowledge` as the primary graph tool backend.
- [ORB-00377] allocated ADR-0195, superseded ADR-0188, and moved long-lived MCP graph reads to a watcher-backed sync policy.
- [ORB-00385] allocated ADR-0197 and removed the orbit-graph equivalence + benchmark harness, amending ADR-0192.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
