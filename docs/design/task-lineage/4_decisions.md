---
summary: "Task Lineage — Decisions"
type: design
title: "Task Lineage — Decisions"
owner: claude
last_updated: 2026-05-09
status: Draft
feature: task-lineage
doc_role: decisions
tags: ["task-lineage"]
---

# Task Lineage — Decisions

ADR log for the task-lineage feature. Format follows [docs/design/CONVENTIONS.md §4](../CONVENTIONS.md): each entry is `Context · Decision · Consequences`, every entry names at least one Cost, numbers are append-only.

ADR numbers are local to this folder. Cross-folder references (e.g., to `knowledge-graph/4_decisions.md`) use full paths. ADRs whose status is `Proposed` flip to `Accepted` when their implementing task lands; the implementing task's ID is appended to the Status line.

---

## ADR-001 — Restore code-graph task attribution as the lineage consumer

**Status:** Proposed · 2026-05 · *implementing task TBD*
**Supersedes:** [knowledge-graph/4_decisions.md ADR-029](../knowledge-graph/4_decisions.md) · [T20260506-11]

**Context.** [Knowledge-graph ADR-029](../knowledge-graph/4_decisions.md) removed graph task attribution citing a 10-day audit with 961 `orbit.graph.*` calls and 0 reverse-lookup uses. The audit was correct in its window — there was no consumer of reverse lookup. Lineage is the consumer the audit didn't have: the symbol-grain `co-touches-symbol` edges, the `feature` closure ([2_design.md §5.1](./2_design.md)), and the symbol-biography renderer ([2_design.md §6](./2_design.md)) all require per-node `task_ids` attribution.

**Decision.** Restore symbol-grain task attribution on knowledge-graph nodes, narrowed to what lineage's derivation pipeline consumes: `task_ids: Vec<String>` (set, not ordered) on each node, populated by a single pass over commits in the build window. Drop the historical hunk-mapping walker, the `structural_conflict` field, and the task-commits sidecar — those carried cost (per ADR-029, the dominant cost was hunk-coordinate remapping after rename hops) without serving lineage's consumers. Flip ADR-029 to `Superseded by [this ADR + implementing task]` when the implementation lands.

**Consequences.**
- Lineage's `kg-attribution` deriver becomes a thin reader of node `task_ids` rather than reinventing attribution.
- `orbit.graph.history` graduates from compatibility stub back to a real graph tool, with shape narrowed to what lineage needs (forward and reverse, file and symbol, no per-hunk).
- The audit's "0 reverse-lookup uses" finding stays valid for the *old* shape; the new shape has exactly one well-defined consumer at rollout, with kill-criterion below.
- **Kill criterion.** If after 90 days post-rollout the lineage authoring-assist hook fires fewer than 5 times per active workspace per week (median), revert this ADR rather than invent more consumers to justify the cost. The rollback is mechanical: drop the `task_ids` field from node serialization, restore the compatibility stub, mark this ADR `Superseded` by the rollback task.
- Cost: build-time pays for the attribution pass on every graph rebuild — bounded but non-zero. Migration cost: existing graph refs without attribution rebuild once, then carry the new field. Schema-shape risk: if a future deriver wants per-hunk attribution, ADR-029's fuller machinery has to be re-introduced (avoidable now, expensive later if needed).

---

## ADR-002 — Typed edges over flat `related_task_ids`

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** The simplest schema for inter-task linkage is a single `related_task_ids: Vec<String>` field on the task record. That shape is universal in issue trackers (Jira "links" panel, GitHub "linked issues"). It also has a universal failure mode: an undifferentiated relation set means consumers can't reason about *kinds* of relationships (supersession vs co-touch vs runtime-parent), so closure operations either treat all edges identically (loss of signal) or rely on text-parsing each edge's surface (fragile).

**Decision.** Edges are typed. The Phase 1 edge type set in [2_design.md §1.1](./2_design.md) (7 task-task plus 4 task-code) forms the system's causal vocabulary; additional types named in that section are deferred. Each edge carries `(from, to, type, source, confidence, evidence, derived_at)`. Closure operations parameterize on edge-type sets so different consumers can dial in a different filter.

**Consequences.**
- Consumers can reason about edge *meaning*, not just edge *presence*. "Tasks that supersede this one" is a different query from "tasks that co-touch this one."
- Adding a new type after rollout requires every consumer to handle it gracefully or be updated. We chose 12 types up front (a deliberate over-shoot) to minimize post-rollout schema churn — see [3_vision.md §1.3](./3_vision.md) for the open question on evolution.
- The taxonomy itself becomes load-bearing documentation: agents authoring tasks learn the edge vocabulary as part of authoring.
- Cost: any scheme where edges must be classified at derivation time is more code than a flat link list. The `task-text` deriver's verb upgrade rules ([2_design.md §2.2](./2_design.md)) are an explicit complexity tax we pay for typed-edge fidelity. Edge-type taxonomy churn is more painful than untyped-link-list evolution.

---

## ADR-003 — Derivation-first edges; declared edges are the exception

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** Hand-authored relation graphs in issue trackers are universally sparse. The Linear/Jira "related issues" field is empty in 80%+ of records by anecdote; the literature on issue-tracker mining exists precisely because the manual link graph is too sparse to be queryable. Asking Orbit's authoring agents to populate a `related_task_ids` field reliably reproduces the failure mode the prior art suffers from.

**Decision.** Edges are *derived first* from three signal sources in the minimal Phase 1 ([2_design.md §2.1](./2_design.md)): `commit-grep`, `kg-attribution`, `task-text`. Three additional derivers (`adr-text`, `runtime-link`, `review-cite`) are deferred to follow-up phases when their consumers ship. Declared edges are accepted only when no derivation source can produce them.

**Consequences.**
- The graph is dense from day one, before any human or agent does any link curation.
- The deriver pipeline becomes the load-bearing component of lineage's correctness story. Bugs in derivation produce wrong edges system-wide; correctness improvement happens at the deriver, not at the edge.
- Provenance per edge ([2_design.md §1.2](./2_design.md)) means wrong edges are flagged into per-source feedback rather than corrected one row at a time.
- Cost: writing six derivers is more upfront work than letting humans link manually. Confidence calibration becomes a real problem (see [3_vision.md §1.1](./3_vision.md)) that hand-authored systems sidestep by treating every link as 1.0 confidence.

---

## ADR-004 — SQLite edge table, not per-task YAML

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** Two storage shapes were on the table. Shape A: edges as fields on the existing per-task YAML (e.g., `edges: { supersedes: [T...], extends: [T...], ... }`). Shape B: a SQLite table separate from the task record. Shape A is consistent with the existing layered store pattern (YAML + SQLite) and keeps task metadata co-located. Shape B requires a new table and a new write path.

**Decision.** Shape B. Edges live in `task_edges` and `task_code_edges` SQLite tables ([2_design.md §1](./2_design.md)). The per-task YAML stores no edge fields beyond the existing `dependencies` (reused for declared blocker edges).

**Consequences.**
- Closure queries are recursive CTEs on a real index, with millisecond latency at expected workspace scales.
- Task YAML stays small and human-readable. Derived edges, which can run into the thousands per workspace, never bloat task files.
- Cursor state, derivation timestamps, and provenance evidence live alongside the edges and don't pollute task records.
- Reading edges does not require reading task YAML; reading task YAML does not require reading edges. The two surfaces are independently consumable.
- Cost: the storage layout carries one more table and one more write path. Backup, sync, and migration logic must cover both. Reviewers who want to see all of a task's relationships must consult two surfaces, not one — mitigated by `orbit.task.show` rendering edges inline as a derived field in JSON output (write-only on the YAML, read-only on the rendering).

---

## ADR-005 — Lineage is workspace-local; cross-machine reach via external_refs render

**Status:** Proposed (cross-machine render deferred from minimal Phase 1) · 2026-05 · *implementing task TBD*

**Context.** Orbit task IDs are workspace-local per [task-sync 2_design.md](../task-sync/2_design.md) and per [knowledge-graph ADR-029](../knowledge-graph/4_decisions.md)'s framing. A "global" lineage graph that merges two engineers' machines requires distributed-consistency machinery far beyond lineage's scope. But cross-engineer presentation is a real need — PR descriptions, audit reports, and shared root-cause analyses want to surface lineage to readers on other machines.

**Decision.** Lineage chains stay strictly local. The workspace-local commitment is load-bearing for the minimal Phase 1 (it is why the substrate avoids distributed-state machinery entirely). The render-time `external_refs` projection that surfaces cross-engineer output (PR descriptions, audit reports) is **deferred from the minimal first draft** — it is a presentation concern, not a substrate concern, and its specification waits until a consumer that emits cross-engineer output ships.

**Consequences.**
- The local graph stays dense, fast, and consistent. No distributed-state CRDT, no cross-machine sync, no eventual-consistency reasoning. (This part is in scope for the minimal Phase 1.)
- Cross-engineer presentation will eventually use external IDs as the cross-machine spine ("[ENG-1234] (T20260510-17 on author-machine)"). The render shape is named here as a placeholder; the actual implementation lands with the first consumer that needs it.
- A workflow that genuinely needs to merge two machines' lineage (e.g., shared root-cause analysis) requires an explicit publish/import step, not a live distributed graph. [3_vision.md §1.9](./3_vision.md) names the open question on what the publish surface looks like.
- Cost: cross-engineer reasoning loses transitive closure. If engineer A's machine has the chain A1→A2 and engineer B's machine has B1→B2, neither machine sees the merged A1→A2↔B1→B2 chain even when both pairs share an external_ref. Acceptable for development-time tools; not acceptable if lineage ever becomes a release-time integrity surface (which would require reconsidering this ADR).

---

## ADR-006 — Closure operations as primary surface; raw edges as escape hatch

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** Two API shapes are common for graph systems. Shape A: low-level traversal primitives (vertex, edge, neighbor) with consumers composing their own queries. Shape B: named, opinionated closures (`lineage`, `feature`, `decision`, `reversal`, `risk`) with raw enumeration as an escape hatch. Shape A is more flexible; Shape B is more legible.

**Decision.** Shape B. The named closures in [2_design.md §5](./2_design.md) are the primary agent-facing surface — minimal Phase 1 ships `feature` and `biography`; additional closures (`closure`, `decision`, `reversal`, `risk`) are deferred and ship with their consumers. `orbit.lineage.edges` exists as a raw escape hatch but is not advertised in the standard skill or in the primary tool reference. Closures default to opinionated parameters (edge-type sets, depth limits, confidence thresholds) chosen to fit each use case.

**Consequences.**
- Agents reason about *named closures* with stable contracts, not about graph mechanics. The skill instructions for `orbit-create-task` can say "compute lineage closure" without specifying which edge types or depth — the closure tool knows.
- Adding a new use case is adding a new named closure, which is more visible and reviewable than letting consumers compose ad-hoc traversals.
- Closure parameter defaults become load-bearing UX decisions; bad defaults silently degrade every consumer.
- Cost: any consumer with a use case the five closures don't cover has to fall back to `edges` and write traversal logic. A consumer that *should* be a sixth named closure but ships as a raw-edges consumer is a soft drift that's hard to detect. Mitigated by treating the named-closure list as a public-API growth list, with new closures requiring an ADR addition (this one).

---

## ADR-007 — Bipartite graph: tasks and code as one structure, not two stitched

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** Two architectural shapes were on the table. Shape A: a standalone task lineage graph that *references* code-graph nodes via stable IDs in evidence fields, with the two graphs traversed separately by consumers. Shape B: an explicit bipartite graph with task↔task edges and task↔code edges as first-class peers, queryable as one substrate.

**Decision.** Shape B. The task graph and the code knowledge graph are one bipartite graph. `task_code_edges` is a peer table to `task_edges` ([2_design.md §1](./2_design.md)). Closure operations cross the two planes freely. The KG remains the authoritative source for code-node identity (stable_id, structure); the lineage system owns only the attribution edges.

**Consequences.**
- Cross-plane queries are first-class. "What code did the lineage of this symbol's tasks touch?" is one CTE, not two queries with manual join.
- The lineage system's correctness is partly the KG's correctness ([2_design.md §8](./2_design.md) names this honestly). A KG attribution bug becomes a wrong lineage edge.
- The bipartite shape is what makes lineage's ambition load-bearing rather than aspirational ([3_vision.md §3.1](./3_vision.md)). Without it, lineage is just typed Jira links.
- Cost: lineage's evolution couples to KG evolution. Schema changes on either side require coordination. Workspace-local store contains two large derived indices instead of one — disk and rebuild cost both grow.

---

## ADR-008 — Symbol-biography surface is the minimal Phase 1 read surface

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** The first-draft scope was reduced to its minimal core — substrate (derivation, bipartite bridge, storage) plus a single read surface. Two candidate read surfaces were on the table: (a) the symbol-biography surface that renders an artifact's history on load, or (b) the authoring-assist hook that runs at task creation. Only one fits the "minimal substrate proof" framing.

**Decision.** The symbol-biography surface (`orbit.lineage.biography`, the §6 renderer, and the `feature` closure substrate) is the minimal Phase 1 read surface. Authoring assist is **deferred** — it is one downstream consumer of the biography substrate and ships in a follow-up phase. Stale-task detection, ADR auto-supersession, and the auto-fire-on-PR "review assist" hook are also deferred (the last is explicitly removed; on-demand `orbit.lineage.reversal` covers reviewer-initiated cases when those closures eventually land).

**Consequences.**
- The minimal Phase 1 ships only one consumer (the biography read surface) of the substrate. That consumer is *the* read surface that operationalizes the "oral history for agents" vision; the kill criterion in ADR-001 binds to biography-surface usage.
- Authoring assist, which would be the first *write-back* consumer, lands later. The minimal first draft proves the substrate carries weight before any write-back consumer commits to it.
- Cost: the minimal Phase 1 has no write-back proof. If the biography surface drives usage but no downstream write-back ever materializes, the substrate is read-only forever. That outcome is acceptable — read-only oral history is still load-bearing — but the design must be honest that "consumers will follow once the substrate ships" is a bet, not a guarantee.

---

## ADR-009 — Deterministic, templated renderer in Phase 1; LLM summarization deferred

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** The biography surface (§6 of [2_design.md](./2_design.md)) renders structured edge data into a prose narrative the agent reads in one shot. Two rendering shapes were on the table. Shape A: a templated renderer that pulls strings from canonical task fields and arranges them per a fixed grammar derived from the edge taxonomy. Shape B: an LLM summarizer that paraphrases the structured data into more readable prose. Shape B is more polished; Shape A is more auditable.

**Decision.** Shape A in Phase 1. The renderer is deterministic and templated — no LLM in the path. Each rendered line traces back to a specific edge and a specific source row (commit SHA, ADR section, task field). Shape B is deferred to Phase 3 behind a feature flag, and only after the deterministic renderer has shipped and the substrate has proved load-bearing.

**Consequences.**
- Every claim in a rendered biography is auditable. An agent that doubts a sentence can resolve the underlying edge and read the source row directly.
- Wrong derivations show up in the biography as wrong sentences, and the fix is at the deriver — not at the renderer, not at the edge row. This preserves the feedback loop named in [ADR-003](#adr-003--derivation-first-edges-declared-edges-are-the-exception).
- Biographies will read more stiffly than an LLM summary would. For a feature whose target consumer is *agents* (not humans skimming a sidebar), this is acceptable; agents do not need polish to absorb context.
- Cost: a templated renderer is more rigid. Some cross-task narrative arcs that would read naturally as paraphrase ("the team walked back from approach X over three tasks") render mechanically as three separate paragraphs. The polish gap is real and is the price of audit fidelity. If the gap proves a usability blocker after Phase 1 lands, the Phase 3 LLM-summarized layer becomes the resolution, gated on the deterministic renderer continuing to be the source of truth underneath.

---

## ADR-010 — Symbol identity stability is a load-bearing assumption; failure modes are named, not designed away

**Status:** Proposed · 2026-05 · *implementing task TBD*

**Context.** The biography surface keys every story on a KG `stable_id`. If `stable_id` does not survive the structural changes lineage is meant to outlive (renames, file moves, refactors), biographies orphan and the surface degrades silently. Per [knowledge-graph ADR-010](../knowledge-graph/4_decisions.md), `stable_id` is intended to survive renames and file moves; per [2_design.md §7.2](./2_design.md), it is known *not* to survive extract-into-multiple, inline-into-caller, cross-language ports, and wholesale rewrites. Two responses were on the table. Response A: invest in KG identity-tracking heuristics (similarity matching, AST-shape stitching) to close the orphan cases before lineage ships. Response B: ship lineage on top of today's KG identity, name the orphan cases in the renderer's output, and treat orphan-rate reduction as a follow-up KG investment.

**Decision.** Response B. Lineage's biography surface ships on top of today's KG `stable_id` semantics. The renderer surfaces the orphan failure mode explicitly via a footnote when a symbol's biography is shorter than its containing-file biography ([2_design.md §7.4](./2_design.md), §6.2). Mitigations on the lineage side (cross-rename evidence accumulation, path-grain fallback, manual `same_as` stitch) cover the most common cases. Investment in KG identity heuristics is a follow-up gated on the measured orphan rate ([2_design.md §9.4](./2_design.md)).

**Consequences.**
- Lineage's correctness ceiling is bounded by KG's identity-tracking ceiling. This is honest and visible to the agent, not hidden behind the renderer.
- The orphan rate becomes a measurable input that drives future KG investment. If biographies orphan often in practice, the response is to improve KG identity, not to weaken the biography surface.
- Phase 1 ships a feature that is sometimes silently incomplete (when the renderer's footnote heuristic misses an orphan case). This is acceptable for a development-time tool but would not be acceptable for a release-time integrity surface.
- Cost: every Phase 1 user encounters orphan biographies in some cases and has to evaluate the footnote. The biography surface is *not* a guarantee; it is a best-effort with provenance. If the orphan rate proves higher than the [§9.4](./2_design.md) 10% threshold, Phase 1's value proposition takes a hit and the kill criterion in [ADR-001](#adr-001--restore-code-graph-task-attribution-as-the-lineage-consumer) becomes more likely to fire.

---

## Task References

- [T20260506-11] — Removed graph task attribution; supersession candidate ([ADR-001](#adr-001--restore-code-graph-task-attribution-as-the-lineage-consumer)).
- [T20260506-15] — Added `orbit.task.locks.reserve` `files` shape; lineage exemplar.
- [T20260510-17] — Removed agent-facing lock instructions; authoring failure that surfaced lineage as a need.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
