---
summary: "Orbit Graph — Decisions"
type: design
title: "Orbit Graph — Decisions"
owner: claude
last_updated: 2026-08-11
status: Draft
feature: orbit-graph
doc_role: decisions
tags: ["orbit-graph"]
related_features: [knowledge-graph]
---

# Orbit Graph — Decisions

This document preserves the feature's non-obvious decisions and their reasoning.

---

## Graph is a derived index, not a versioned store

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:04.063573Z · [ORB-00294], [ORB-00377], [ORB-10479]

**Context.** `orbit-knowledge` was built as a git-like history layer: content-addressed objects, mutable refs, atomic swaps, lock protocols. In practice the graph is consumed as "fresh queryable index of the current code" — none of the version-store affordances are used by agents.

**Decision.** Reframe the graph as a derived index, regenerable from `(file_contents, extractor_version)`. Delete object storage, mutable refs, and atomic-swap locking. Single SQLite file per worktree is the only durable state.

**Consequences.**
- Deletes ~3k LOC of object-store, lock, and ref-management code.
- Removes the lock protocol's structural inability to coordinate same-branch worktrees (see [Branch-scoped refs over a single shared ref](../knowledge-graph/4_decisions.md#branch-scoped-refs-over-a-single-shared-ref)'s cost line).
- Cost: **no history.** "What did the graph look like at commit X?" is no longer a query the graph can answer. Use git for that.

## Per-worktree DB filename embeds extractor version

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:07.790131Z · [ORB-00294], [ORB-10479]

**Context.** When extractor logic changes (new language, fixed parse bug, schema tweak), the on-disk DB becomes incompatible. The traditional fix is schema migration code; the V1 ethos is to keep complexity out of the storage layer.

**Decision.** DB filename is `<branch>.<extractor_version>.db`. Bumping `EXTRACTOR_VERSION` makes old DBs invisible; they're deleted on next sync. No migration code.

**Consequences.**
- Extractor version bumps are zero-friction; agents never see migration failures.
- Multiple extractor versions can coexist on disk temporarily during rollback testing.
- Cost: **cold rebuild after every extractor bump.** For a 200k LOC repo that's ~3s, acceptable per the perf budget. For a much larger repo it could become noticeable; revisit if a user complains.

## Symbol IDs are ephemeral; resolution by qualified name

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:11.014117Z · [ORB-00294], [ORB-10479]

**Context.** With incremental sync, a file's symbol rows are deleted and re-inserted on change. If cross-file refs FK to `symbols.id`, every incremental rebuild orphans inbound refs from other files. The current `orbit-knowledge` schema has an `identity_key` column trying to paper over this; it doesn't fully work and adds complexity.

**Decision.** No foreign key on `symbols.id` from any table. Refs and relations resolve by `target_qualified` (string lookup). A `target_symbol_hint INTEGER` column exists as a build-time cache but is non-authoritative.

**Consequences.**
- Incremental sync is correct by construction: dropping a file's symbols doesn't dangle anything.
- No `identity_key` column or cross-build lineage tracking machinery.
- Cost: **string lookups instead of integer FK joins.** SQLite's B-tree on `target_qualified` keeps this fast (low single-ms even on 100k symbols), but it's a real cost compared to the natural FK design. Rename tracking is a separate feature on top of git, not a graph affordance.

## Two ref tables, not one

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:14.124143Z · [ORB-00294], [ORB-10479]

**Context.** The original draft put all cross-symbol edges in one `refs` table with a `kind` column covering `call | type | impl | use | trait_bound`. Calls and type uses are anchored to `(file, span)`. Impl relations are anchored to `(concrete_symbol, trait_symbol)` with no useful span. Mixing them forces meaningless columns on the impl side.

**Decision.** Split into `refs` (textual, `from_file + from_span_start/end`) and `relations` (symbol-to-symbol, `from_qualified + to_qualified`). CLI `--kind impl` is a routing alias to `relations`.

**Consequences.**
- "What implements X?" is a single `relations` index lookup, fast enough to be a hot path.
- The two tables are independently extensible (e.g. adding `relations.kind = "annotates"` for TypeScript decorators) without inflating the `refs` shape.
- Cost: **two indexes to maintain instead of one.** Schema is wider; the `refs` command needs to union two underlying queries. Acceptable for the correctness and ergonomics gain.

## Sync policy is a property of the Graph handle

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:17.439519Z · [ORB-00294], [ORB-10479]

**Context.** The original draft hardcoded "10ms stat budget at 5000 files; cache window 500ms" inside the query layer. The budget doesn't scale, and the policy mixes product decisions into the library.

**Decision.** `Graph::open(root, policy: SyncPolicy)` where `SyncPolicy` is `Manual | OnRead | Windowed { window: Duration }`. CLI default: `Manual`. MCP server default: `Windowed { window: 500ms }`.

**Consequences.**
- Tests use `Manual` for determinism; long-lived processes use `Windowed`; one-shot scripts can use `OnRead` for paranoia.
- The library no longer carries an implicit perf contract that breaks silently at scale.
- Cost: **callers must choose.** No "just works" default beyond per-entry-point conventions. The conventions are documented but the choice is exposed.

## Performance gate is against committed baseline, not last run

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:20.765645Z · [ORB-00294], [ORB-10479]

**Context.** A perf regression gate that compares "this run vs previous merged run" ratchets up to whatever the latest measurement happened to be — slow degradation goes undetected.

**Decision.** Baseline lives at `bench/baselines.json`, committed to the repo. Regression gate fires when a run is >20% slower than the *committed* baseline. Bumping the baseline requires a labeled PR and a one-line justification.

**Consequences.**
- Slow erosion is caught; cumulative drift requires an explicit acknowledgment.
- Performance wins are realized by intentional baseline bumps, not silent improvements that immediately become the new floor.
- Cost: **baseline updates are friction.** Every routine improvement requires a labeled PR. Acceptable — the friction is intentional and the alternative (no friction, no guarantee) is worse.

## Use per-commit DB files for detached HEAD

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-05-25 17:28:30.633353Z · [ORB-00331]
**Paths:** `crates/orbit-graph/src/lib.rs`, `crates/orbit-graph/src/store/mod.rs`, `docs/design/orbit-graph/**`

### Context
ORB-00326 (78e26efa) fixed detached-HEAD meta recording, but the filename still used `HEAD.<version>.db`, so ORB-00331 had to choose between keeping one `HEAD` DB with churn warnings or giving each detached commit its own DB file. Concurrent agents on different detached commits need isolation more than they need a single reusable cache file.

### Decision
Use per-commit detached filenames: `detached-<short-sha>.<extractor_version>.db`. Branch-attached checkouts keep the existing `<branch>.<extractor_version>.db` layout, and `meta.branch` remains `HEAD` for detached checkouts.

### Consequences
- Detached checkouts on different commits no longer invalidate each other through the same `HEAD` database.
- The stale-DB sweep must remove detached DBs whose commits are no longer reachable from any local ref, while preserving the active DB family.
- Cost: bisecting or cherry-picking through many detached commits can create O(N) database files until the reachability sweep prunes them.

## Roll back orbit-graph tool cutover to orbit-knowledge

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-08-01 19:15:24.059717Z · [ORB-00344], [ORB-10479]

**Context.** ORB-00338 cut the active graph query tools over from `orbit-knowledge` to `orbit-graph`, but audit data and post-cutover testing found unacceptable steady-state regressions: 13.5x p50 search slowdown, a roughly 9s cold-call floor, deleted high-use tools, incomplete plugin MCP exposure, byte-array `show` output, empty `trace` results for real enum-dispatch commands, and direction-confused `impact` output.

**Decision.** Restore the legacy `orbit-knowledge`-backed `orbit.graph.search`, `show`, `refs`, `callers`, `pack`, `overview`, `implementors`, and `deps` surface as the active backend. Keep the `orbit-graph` crate and equivalence harness in tree, but gate any future cutover on the rollback learnings captured in the global ADR.

**Consequences.**
- Future cutover work must use `SyncPolicy::Manual` as the query-tool default unless a measured long-lived process explicitly opts into another policy.
- Pre-cutover audit-log analysis, plugin MCP exposure equivalence, UTF-8 text response boundaries, trace/impact correctness gates, and cold-call latency measurements are required before another backend swap.
- Lost for now: cutover-only `callees`, `impact`, `trace`, the changed `sync` shape, and the extended graph-equiv corpus.
- Cost: **cutover pauses.** The `orbit-graph` backend remains available for development, but agents lose the new cutover-only APIs until the root causes are fixed and a new cutover passes the gates.

- **[Watcher-backed graph reads](#watcher-backed-graph-reads) — Watcher-backed graph reads** — Accepted.

## Remove the orbit-graph equivalence and benchmark harness

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-06-14 03:00:18.494265Z · [ORB-00385]
**Paths:** `tools/graph-equiv/**`, `bench/**`, `Makefile`, `.github/workflows/ci.yml`, `scripts/check-dependency-direction.sh`, `docs/design/orbit-graph/**`

### Context

[Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge) rolled back the orbit-graph v2 tool cutover to orbit-knowledge and, as one of its consequences, decided to **keep the `orbit-graph` crate and the equivalence harness in tree** to gate a future cutover. The equivalence harness is the `tools/graph-equiv` workspace binary (plus its frozen multi-language corpus) and the `bench/` benchmark-baseline scripts (`baselines.json`, `check_baseline.sh`, `run_graph_bench_ci.sh`, `equiv-waivers.md`). It dual-ran the v1/v2 backends over a frozen corpus and failed CI (`make ci-equiv`, the `graph-equiv` GitHub Actions job) on any diff outside documented tolerances.

In practice the v2 cutover is paused indefinitely ([Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)) and none is scheduled. The harness nonetheless carries standing cost: a Cargo workspace member, a dedicated CI job, a Makefile target, a `check-dependency-direction.sh` guardrail entry, and four documentation references — all maintaining a gate for a migration step that is not active.

### Decision

Remove the equivalence and benchmark harness from the tree: delete `tools/graph-equiv/` (and its frozen corpus) and `bench/`, and unwire them from the build — the Cargo workspace member, the `make ci-equiv` target, the `graph-equiv` CI job, and the dependency-direction guardrail allowlist entry.

This **amends [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)**: it reverses *only* [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)'s "keep the equivalence harness in tree" consequence. [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)'s primary decision — the rollback of the v2 cutover and the gates required before any future cutover — remains fully in force, and the `orbit-graph` crate itself is **kept**. Only the equivalence/benchmark tooling is removed.

The v1↔v2 equivalence relation documented in GRAPH_SPEC still defines what a future cutover must satisfy; if a cutover is rescheduled, the harness is reintroduced fresh as part of that work rather than carried indefinitely as an inactive scaffold.

### Consequences

- The workspace drops one crate and the `graph-equiv` CI job; `make ci-equiv` no longer exists. CI and `cargo check --workspace` are unaffected otherwise.
- The documented equivalence relation (GRAPH_SPEC §migration) becomes plan-only: no in-tree binary enforces it until a cutover is rescheduled.
- [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)'s harness-retention consequence is amended by this ADR; its rollback decision and pre-cutover gates are unchanged.
- Cost: a future v2 cutover must rebuild the equivalence + benchmark harness — binary, frozen corpus, baselines, and CI wiring — from scratch rather than resuming an in-tree scaffold, and the existing frozen corpus and baseline history are lost.

## Cut over to orbit-graph (v2) and decommission orbit-knowledge

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-06-14 21:41:06.694501Z · [ORB-00391]
**Supersedes:** [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)

**Context.** [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge) rolled back the ORB-00338 v2 cutover and restored `orbit-knowledge` (v1) as the active graph backend, gating any future cutover on a set of pre-cutover correctness/latency learnings. ORB-00386/00387/00389/00390 closed those gates against the watcher-backed orbit-graph (v2): UTF-8 `show` boundaries, trace/impact correctness, the `overview`/`implementors`/`deps` navigation queries (restoring v1 parity), and the `refs` fuzzy fallback. The automated equivalence/effectiveness harness that [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)/§16 envisioned was removed by [Remove the orbit-graph equivalence and benchmark harness](#remove-the-orbit-graph-equivalence-and-benchmark-harness) and never rebuilt; with the gates closed, rebuilding it as a precondition carried no proportionate value.

**Decision.** Cut the agent graph surface over to orbit-graph (v2) and remove `orbit-knowledge` entirely. The v2 tools (`sync`, `search`, `show`, `refs`, `callees`, `impact`, `trace`, `overview`, `implementors`, `deps`) are served by the in-process `GraphToolRegistry` in `orbit-mcp`, which activates whenever the host exposes no `orbit.graph.*` schema. The v1 builtins, the `orbit graph` CLI command, the init-time graph build, and the v1 metrics pipeline are removed; the knowledge-stats computation moves to `orbit_core::metrics`. The measurement bar for this cutover is **manual QA plus a v1-vs-v2 spot-check**, accepted in place of the never-rebuilt automated harness. The `pack` tool is dropped rather than ported (ORB-00388 rejected); `callers` is subsumed by `refs`.

**Consequences.**
- orbit-graph is the sole graph surface; `orbit-knowledge` is deleted from the workspace (~24k LOC, plus the v1 builtins and the `orbit graph` CLI command).
- Agents reach the graph only through `orbit-mcp`; there is no `orbit graph` subcommand. The standalone `orbit-graph-cli` binary remains for direct CLI use.
- The dashboard knowledge-stats panel keeps working via `orbit_core::metrics::aggregate`; pack-compression fields degrade to defaults (pack is gone), matching the pre-existing pack-less code path.
- [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)'s rollback decision is reversed; its `SyncPolicy::Manual` default note is superseded by the watcher-backed policy ([Watcher-backed graph reads](#watcher-backed-graph-reads)) for long-lived MCP handles.
- Cost: the v1 content-addressed store, working-graph write surface, and graph-history attribution are gone; there is no automated equivalence gate guarding regressions against the (now-removed) v1 backend.

## Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-07-26 21:51:44.519954Z · [ORB-00396], [ORB-10458]

### Context

[Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge) cut the agent graph surface to orbit-graph (v2) and, in doing so, removed the `orbit graph` CLI command — agents reach the graph in-process over MCP, and direct CLI users were pointed at the standalone `orbit-graph-cli` binary. In practice that binary is not always on `PATH` (the agent shell documented in [`plugin/agents/orbit-code-reader.md`](../../../plugin/agents/orbit-code-reader.md) notes "`orbit-graph-cli` is not on PATH in this environment"), leaving a shell user who holds only the `orbit` binary with no command-line path to the graph. Every other Orbit capability is reachable from the single `orbit` binary; the graph was the lone exception.

### Decision

Reintroduce `orbit graph` as a thin wrapper over the `orbit-graph-cli` command layer. `orbit-graph-cli` is lib-ified (lib + bin): its `Command` subcommand enum and `Command::run` dispatch move into a library surface that both the standalone binary and `orbit-cli` consume, so there is exactly one command layer and no duplication. `orbit-cli` embeds that enum under an `orbit graph` parent and prints the same JSON the standalone binary emits, mapping the graph CLI error into `OrbitError`. The graph subcommands stay worktree-scoped (the DB is discovered from the current git worktree) and do not route through `OrbitRuntime`. This amends only [Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge)'s "there is no `orbit graph` subcommand" consequence; the v2 cutover, the MCP adapter as the agent surface, and the removal of `orbit-knowledge` are unchanged.

### Consequences


- `orbit graph {sync, search, show, refs, callees, impact, trace, overview, implementors, deps, version, db-path, clean}` is available from the single `orbit` binary; output matches the standalone `orbit-graph-cli` (same library, same compact JSON).
- New crate edge `orbit-cli → orbit-graph-cli` (recorded in [`ARCHITECTURE.md`](../../../ARCHITECTURE.md)). `orbit-graph-cli` now publishes a minimal library surface (`Command`, `Command::run`, `CliError`); the per-subcommand arg structs are made `pub` to keep the public enum's interface clean under `-D warnings`.
- The agent-facing graph surface is unchanged: agents still use the in-process MCP adapter, not `orbit graph`. The new subcommand is for humans/scripts holding the `orbit` binary.
- Cost: a second consumer of the orbit-graph-cli command layer means a subcommand change now ripples to two front ends' help/output expectations. The duplication-free lib split confines the implementation to one edit site, but the orbit-cli parse tests and any `orbit graph` doc references must track the surface.

**Note (ORB-10357, 2026-07-25).** Daniel redirected: fold `orbit-graph-extract` and `orbit-graph-cli` into `orbit-graph` (it doesn't have much use and will eventually be phased out) and remove the `orbit graph` subcommand from `orbit-cli` entirely. This reverses this ADR's decision: the `orbit-cli → orbit-graph-cli` edge and the `orbit graph` subcommand it introduced are removed, not merely amended. The former `orbit-graph-cli` command layer (`Cli`, `Command`, `CommandContext`) is folded into `orbit-graph` as a `cli` module with no external caller — parked pending the graph crate's eventual deletion, per the note in [1_overview.md](./1_overview.md). No standalone binary was introduced for the consolidated crate.


## Consolidate the Selector parser into orbit-common

**Recorded:** 2026-07-04 10:16:40.110359Z · [ORB-10011]

Backfill allocation: this ID was already used by the committed docs/design/orbit-graph/4_decisions.md entry (ORB-10011, Selector parser consolidation into orbit-common::utility::selector) but the allocation record was missing from this workspace ADR store. Re-pointed during ORB-10016 to keep the store and docs consistent; the orbit-cmd extraction ADR was re-allocated as [Extract the CLI-facing command layer into orbit-cmd](../../orbit-core/4_decisions.md#extract-the-cli-facing-command-layer-into-orbit-cmd).

## Watcher-backed graph reads

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-06-13 18:14:06.367874Z · [ORB-00377]
**Supersedes:** [Sync policy is a property of the Graph handle](#sync-policy-is-a-property-of-the-graph-handle)
**Paths:** `crates/orbit-graph/**`, `crates/orbit-mcp/**`, `docs/design/orbit-graph/**`, `ARCHITECTURE.md`

**Context.** ORB-00377 found that the MCP `orbit.graph.*` read path was effectively poll-on-read: the 500ms `Windowed` policy elapsed between most agent calls, so each query paid for a full worktree diff before running the SQLite lookup. Lengthening the window would reduce frequency but would keep query latency coupled to repository size.

**Decision.** Long-lived MCP graph handles use a watcher-backed policy: `Graph::open` performs one initial auto sync, starts a `notify` watcher scoped to the worktree, coalesces relevant filesystem events behind a debounce, and runs sync in the background. Query methods do not run inline sync for this policy; they read from a cached SQLite connection. The freshness contract is eventual: after a same-process file edit, graph reads may remain stale until the watcher observes and syncs the event, normally within the debounce plus sync duration; callers needing a hard read-after-write barrier must call `Graph::sync`/`orbit.graph.sync` before querying.

**Consequences.**
- Repeated graph reads with no intervening edits are pure SQLite lookups and do not initiate scanner walks.
- Watcher overflow or watcher errors request a coalesced auto sync, preserving the conservative fallback path.
- `Windowed` remains available as an explicit fallback policy, but it is no longer the MCP default.
- Cost: the MCP process now depends on platform filesystem watcher behavior and may serve stale graph data during the documented debounce-plus-sync window.

## orbit-graph is CLI-surface only; separate MCP deferred until a shell-less consumer exists

**Superseded by:** [Retire and delete Orbit's code-graph subsystem](#retire-and-delete-orbits-code-graph-subsystem)
**Recorded:** 2026-07-19 07:57:50.649679Z · [ORB-10325]

### Context

The code graph is exposed via the `orbit graph` CLI (`orbit-graph-cli`, structured JSON on stdout/stderr) and 10 `orbit.graph.*` MCP tools served by `orbit-remote/src/mcp/graph.rs` (~588 LOC, `GraphToolRegistry`, 10 schemas) on the broker composition. Investigation during ORB-10325 established the true topology: there is ONE broker surface serving both engine roles and local editors identically; the hub (the only real remote surface) already registers graph `recognition_only` and Bridge omits graph entirely. No separable "external exposure" exists at the broker.

Guiding lesson (2026-07-18): adopt a tool for the smallest footprint where it's the best option — extra capability doesn't justify the extra space it claims.

### Decision (second amendment, 2026-07-19)

Full removal restored. Remove the `orbit.graph.*` MCP tools from the broker surface entirely; `orbit graph` CLI (structured JSON) becomes the sole graph surface.

The first amendment kept in-process serving because the planning-duel planner/arbiter roles are shell-less and their instructions mandate graph navigation. Daniel's ruling: that dependency is prompt-imposed, not intrinsic — planning duels do not need call-graph-verified precision at authoring time; blast-radius verification belongs to the implement/review activities, which have shell and can run `orbit graph` directly. Accordingly:

1. Rewrite `PLANNING_DUEL_INSTRUCTION` and `ARBITER_INSTRUCTION` (`orbit-engine/.../planning_duel/roles.rs`) to plan and adjudicate via `fs.read` and search-level navigation, dropping the mandates that require graph tools; remove the 8 graph tool grants from `planner_activity`/`arbiter_activity`.
2. Remove `graph.rs`, the broker `advertised(graph)` and hub `recognition_only(graph)` registrations, graph entries in `canonical_mcp_tool_definitions`/`safe_mcp_tool_names`, schema metadata, and the graph-exercising tests/snapshots.
3. Remove graph entries from `tool_allowlist.rs` (all in-process consumers are gone once duel roles and vestigial grants are cleaned); drop vestigial grants in `dispatch_agent.yaml`/`epic_orchestrator.yaml`; repoint `deterministic_reference.yaml`'s fixture.
4. Shell-holding activities (agent_implement, agent_review, step_failure_recovery) use the `orbit graph` CLI via `proc.spawn`, as they already can.

Do not build a separate graph MCP server. The trigger remains an external consumer with MCP access but no shell; none exists.

### Consequences

- The broker sheds 10 tool schemas from every connecting client's tools/list (engine roles and local editors alike), ~588 LOC of serving code, and the graph test/snapshot surface; the `GraphToolRegistry` cache/staleness class disappears.
- Planning-duel planner/arbiter lose symbol-graph navigation; their instructions are correspondingly rewritten to a plan-level standard of evidence. Risk accepted by Daniel: plan precision claims are verified downstream at implement/review, which retain full graph access via CLI.
- Cost: if duel plan quality measurably degrades without graph navigation, the remedy is granting those roles a narrowly scoped capability or revisiting this ADR — a deliberate future decision, not a silent re-add.
- Cost: any future shell-less consumer requires building the deferred separate MCP server.

## Retire and delete Orbit's code-graph subsystem

**Recorded:** 2026-07-27 00:25:15.600223Z · [ORB-10357], [ORB-10473], [ORB-10491]
**Supersedes:** [Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge), [orbit-graph is CLI-surface only; separate MCP deferred until a shell-less consumer exists](#orbit-graph-is-cli-surface-only-separate-mcp-deferred-until-a-shell-less-consumer-exists), [Reintroduce orbit graph as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli), [Use per-commit DB files for detached HEAD](#use-per-commit-db-files-for-detached-head), [Watcher-backed graph reads](#watcher-backed-graph-reads), [Remove the orbit-graph equivalence and benchmark harness](#remove-the-orbit-graph-equivalence-and-benchmark-harness), [Graph is a derived index, not a versioned store](#graph-is-a-derived-index-not-a-versioned-store), [Per-worktree DB filename embeds extractor version](#per-worktree-db-filename-embeds-extractor-version), [Symbol IDs are ephemeral; resolution by qualified name](#symbol-ids-are-ephemeral-resolution-by-qualified-name), [Two ref tables, not one](#two-ref-tables-not-one), [Sync policy is a property of the Graph handle](#sync-policy-is-a-property-of-the-graph-handle), [Performance gate is against committed baseline, not last run](#performance-gate-is-against-committed-baseline-not-last-run), [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge)
**Paths:** `crates/orbit-graph/**`, `crates/orbit-core/src/command/task/**`, `crates/orbit-core/src/config/**`, `crates/orbit-cmd/src/doctor.rs`, `crates/orbit-cli/src/command/doctor.rs`, `crates/orbit-dashboard/src/health.rs`, `benchmarks/graph/**`, `benchmarks/graph-latency/**`, `docs/design/orbit-graph/**`, `docs/runbooks/**`, `docs/CONFIG.md`

### Context

Orbit's code graph grew into a large, specialized subsystem whose maintenance and product surface no longer justified its utility. ORB-10357 removed the `orbit graph` command and all agent-facing graph surfaces, consolidated the remaining implementation into a dependent-free `orbit-graph` crate, and explicitly parked that crate for deletion. The active runtime still contains several vestiges: symbol `context_files` emit a graph-availability warning despite validating only their file anchor, `orbit doctor` and detailed dashboard health probe graph databases that Orbit can no longer build or use, stale graph databases may remain under `.orbit/graph` and `.orbit/knowledge/graph`, `[graph] editing` is still accepted as inert configuration, and graph-specific benchmark suites remain in tree.

Older accepted decisions—[Use per-commit DB files for detached HEAD](#use-per-commit-db-files-for-detached-head), [Watcher-backed graph reads](#watcher-backed-graph-reads), [Remove the orbit-graph equivalence and benchmark harness](#remove-the-orbit-graph-equivalence-and-benchmark-harness), [Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge), [orbit-graph is CLI-surface only; separate MCP deferred until a shell-less consumer exists](#orbit-graph-is-cli-surface-only-separate-mcp-deferred-until-a-shell-less-consumer-exists), and [Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli)—describe graph storage, watcher, benchmark, or product surfaces that the retirement removes. [Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli)'s body records the product-surface reversal, but its lifecycle status and the other records were not fully reconciled.

### Decision

Retire the code graph as an Orbit capability and delete the remaining subsystem rather than preserve dormant compatibility surface.

1. Orbit has no graph CLI, MCP tools, tool-registry entries, execution guidance, health/readiness checks, or runtime behavior.
2. A `symbol:` task context selector is a canonical selector whose file anchor is checked for workspace containment and existence. Orbit does not resolve or validate symbol existence, and it emits no graph-availability warning.
3. `orbit doctor` does not diagnose graph indexes. It may expose the explicit, opt-in `--remove-graph` maintenance action to delete retired graph state from the exact workspace locations `.orbit/graph` and `.orbit/knowledge/graph`; ordinary doctor runs remain read-only and graph-unaware.
4. The dependent-free `orbit-graph` crate, the inert `[graph] editing` configuration key, and the graph-specific `benchmarks/graph/` and `benchmarks/graph-latency/` suites are deletion targets, not compatibility commitments. Non-graph benchmark suites remain. Physical deletion is a separate bounded task from the selector/doctor cleanup so each change remains reviewable.
5. Historical graph design documents may remain archived for provenance, but active documentation must not describe graph as a supported Orbit capability.

This decision supersedes [Use per-commit DB files for detached HEAD](#use-per-commit-db-files-for-detached-head), [Watcher-backed graph reads](#watcher-backed-graph-reads), [Remove the orbit-graph equivalence and benchmark harness](#remove-the-orbit-graph-equivalence-and-benchmark-harness), [Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge), [orbit-graph is CLI-surface only; separate MCP deferred until a shell-less consumer exists](#orbit-graph-is-cli-surface-only-separate-mcp-deferred-until-a-shell-less-consumer-exists), and [Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli).

### Consequences

- Orbit's supported navigation path is filesystem reading/search plus `orbit search`; call-graph queries are no longer provided.
- Task selector behavior becomes honest and deterministic: symbol fragments remain descriptive metadata while validation is file-anchor based.
- Health output no longer reports an unavailable subsystem, and operators receive one explicit cleanup path for retired graph data.
- Removing the isolated crate, graph-only benchmark suites, and inert configuration reduces code, dependencies, configuration, and maintenance burden without adding a replacement abstraction or feature flag.
- Cost: Orbit loses built-in call-graph navigation, the ability to reuse old graph databases, and the in-tree reproducible graph benchmark artifacts. This is accepted because the graph has no remaining callers and its observed utility does not justify its footprint; git history and ADRs retain provenance.
- Cost: `--remove-graph` permanently deletes derived graph state. The action is opt-in, targets only fixed Orbit-owned directories, and is idempotent; no deletion occurs during an ordinary doctor run.

## Task References

- [ORB-00294] allocated the six initial orbit-graph ADR IDs ([Graph is a derived index, not a versioned store](#graph-is-a-derived-index-not-a-versioned-store) through [Performance gate is against committed baseline, not last run](#performance-gate-is-against-committed-baseline-not-last-run)).
- [ORB-00331] allocated [Use per-commit DB files for detached HEAD](#use-per-commit-db-files-for-detached-head) and shipped the detached-HEAD per-commit DB layout.
- [ORB-00344] allocated [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge) and restored `orbit-knowledge` as the primary graph tool backend.
- [ORB-00377] allocated [Watcher-backed graph reads](#watcher-backed-graph-reads), superseded [Sync policy is a property of the Graph handle](#sync-policy-is-a-property-of-the-graph-handle), and moved long-lived MCP graph reads to a watcher-backed sync policy.
- [ORB-00385] allocated [Remove the orbit-graph equivalence and benchmark harness](#remove-the-orbit-graph-equivalence-and-benchmark-harness) and removed the orbit-graph equivalence + benchmark harness, amending [Roll back orbit-graph tool cutover to orbit-knowledge](#roll-back-orbit-graph-tool-cutover-to-orbit-knowledge).
- [ORB-00391] allocated [Cut over to orbit-graph (v2) and decommission orbit-knowledge](#cut-over-to-orbit-graph-v2-and-decommission-orbit-knowledge), cut the agent graph surface over to orbit-graph (v2), and decommissioned `orbit-knowledge`.
- [ORB-00396] allocated [Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli), lib-ified `orbit-graph-cli`, and reintroduced `orbit graph` as a thin CLI wrapper over it.
- [ORB-10011] allocated [Consolidate the Selector parser into orbit-common](#consolidate-the-selector-parser-into-orbit-common) and consolidated the `Selector` parser into `orbit-common::utility::selector`.
- [ORB-10357] superseded [Reintroduce `orbit graph` as a thin wrapper over orbit-graph-cli](#reintroduce-orbit-graph-as-a-thin-wrapper-over-orbit-graph-cli): removed the `orbit graph` subcommand from `orbit-cli` and folded `orbit-graph-extract`/`orbit-graph-cli` into `orbit-graph`, leaving it dependent-free.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
