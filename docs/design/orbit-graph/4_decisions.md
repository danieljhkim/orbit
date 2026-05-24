---
summary: "Orbit Graph — Decisions"
type: design
title: "Orbit Graph — Decisions"
owner: claude
last_updated: 2026-05-24
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

**Status:** Proposed · 2026-05 · [ORB-00294]

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

---

## Task References

- [ORB-00294] allocated the six initial orbit-graph ADR IDs (ADR-0184 through ADR-0189).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
