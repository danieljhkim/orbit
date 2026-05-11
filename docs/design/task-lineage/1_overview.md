# Task Lineage — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-09

Task lineage is **durable agent memory keyed on code structure**. Every symbol in a codebase has a history — a sequence of decisions, reversions, supersessions, and intent shifts that produced its current shape. Today that history lives partly in commit messages (which fragment under refactors), partly in task records (which agents rarely read transitively), partly in ADRs (which bind to other ADRs but not to symbols), and mostly in the heads of the humans who lived through it. When an agent walks into a fresh session, it has none of it.

Lineage is a typed, bidirectional graph that binds Orbit's task store to its code knowledge graph and treats commits, task-record text, and code-graph node attribution as evidence on edges between them. The graph hydrates each KG symbol with a **biography** — a chronologically-ordered, edge-typed narrative of the tasks that touched it. An agent that loads a symbol gets the symbol's story in one shot, not as a graph dump but as readable history. Edges are *derived* from existing signals, not hand-authored.

The minimal first draft ships the **edge store**, **three derivers** (`commit-grep`, `kg-attribution`, `task-text`), the **bipartite task↔code bridge**, the **`feature` closure**, and the **symbol-biography renderer** that turns it all into an agent-readable narrative. Everything else — additional derivers, additional closures, authoring assist, stale-task detection, ADR auto-supersession, cross-machine rendering — is **deliberately deferred** to follow-up phases. The first draft proves the substrate carries weight; consumers and polish wait until that proof lands. Forward-looking sketches of the deferred pieces live in [3_vision.md](./3_vision.md).

This document is the entry point. [2_design.md](./2_design.md) specifies the edge schema, derivation pipeline, storage layout, renderer, and tool surface; [3_vision.md](./3_vision.md) names open questions and ambitious projections; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

### 1.1 The core problem: agents lack persistent symbol-grain context

A human engineer carries a feature's history in their head. They remember why the lock-reservation API has the shape it does, which earlier approach it replaced, what reviewer pushed back on which assumption, which ADR governs the surrounding code. None of that is written down in a form an agent can query in one shot — it lives in the engineer's *memory of having lived through it*.

Agents start each session with no such memory. They can read the file in front of them, grep the codebase, and call `git blame`, but they cannot — without considerable effort — reconstruct the *narrative* of how a symbol came to look the way it does. Every Orbit session re-pays the discovery cost the human paid once, and pays it incompletely, because most agents triage instead of exhaustively walking.

The cost shows up as a recurring failure mode: an agent makes a locally-correct change that contradicts a still-Accepted ADR, or removes a feature without noticing that downstream sales copy still describes it ([T20260510-17] is one such instance — the agent identified the seeded skill files but missed [README.md:27](../../../README.md) and was misled by a stale execution-summary claim in the originating task). The bug class isn't "this one agent missed this one file." The bug class is *agents systematically operating without the institutional context that humans take for granted* — and there is no tool today that gives it to them in one shot.

### 1.2 The substrate exists; the stories are the missing layer

Orbit already stores every piece of evidence the missing context would draw on, but stores them as **disjoint data structures**:

- **Task store** owns descriptions, plans, execution summaries, status history, and `context_files`.
- **Git history** owns commit-level attribution via the `[T...]` convention enforced by [CLAUDE.md](../../../CLAUDE.md).
- **Knowledge graph** owns symbol/file-level structure with stable IDs that survive renames, with a now-removed task attribution layer ([ADR-029](../knowledge-graph/4_decisions.md) / [T20260506-11]).
- **ADR log** owns the supersession chain in `Status` lines ("Superseded by ADR-NNN / [T...]").
- **Run store** owns activity/job parent-child runtime edges (e.g. `task_gate_pipeline` → `task_pr_pipeline`).
- **Review threads** own reviewer cross-citations.

Each lookup works in isolation. None compose. The KG is the structural substrate that should anchor the rest — it already keys on stable symbol identity that survives the refactors, renames, and file moves under which raw commit history fragments — but until lineage hydrates it with task attribution and edges, it carries structure without story.

**Lineage is the projection of those five sources onto a single typed graph, indexed by KG symbol identity, rendered as oral history the agent can absorb in one read.**

### 1.3 Why "oral history" is the right metaphor

Human teams pass institutional context informally — over lunch, in code review comments, in the moment when a senior engineer says "we tried that two years ago, here's why we walked back from it." None of that is written in any single document. It survives because humans are continuous and remember.

Agents are discontinuous and don't. The substitute can't be a wiki an agent might read — agents won't speculatively load wikis that might contain relevant context. It has to be a structured record *attached to the artifact the agent is already loading* (the symbol, the file, the task), surfaced automatically, with provenance the agent can audit when something looks wrong. That is what lineage is.

The metaphor is load-bearing in two ways:

1. **The story is keyed on the artifact, not on time.** Loading `Foo::bar()` produces Foo::bar()'s history, not "this week's story." Symbols carry their own biographies the way long-tenured engineers do.
2. **The story survives structural change.** If `Foo::bar()` gets extracted into `Bar::baz()`, the story has to follow. KG `stable_id` identity is the load-bearing assumption that makes this work; lineage's correctness depends on it (see [2_design.md §7](./2_design.md)).

### 1.4 Why ADR-029's removal needs reversing

[ADR-029](../knowledge-graph/4_decisions.md) removed graph task attribution citing a 10-day audit with 0 reverse-lookup uses. That audit was correct *in its window* — there was no consumer of reverse lookup at the time. Lineage is the consumer the audit didn't have. Without per-node `task_ids`, there is no way to key a symbol's biography on the symbol itself; the entire vision collapses to "task-graph that happens to mention code." This design supersedes ADR-029 with the lineage feature as the consumer of record; see [4_decisions.md ADR-001](./4_decisions.md). The kill criterion in ADR-001 is the honesty mechanism: if the biography surface doesn't see real usage, attribution is removed again.

### 1.5 Why pull-only inspection is not enough

Pull-only inspection (manually opening prior tasks, running `git log --grep`, reading ADRs) imposes the discovery burden on the same agent that needed lineage to compensate for incomplete recall. The agent that doesn't know to ask "what tasks touched this symbol?" is exactly the agent who needs the biography surfaced automatically. **The system must derive edges automatically, render them as readable history, and surface that history at the moment the agent loads the artifact, or it reproduces the failure mode it exists to fix.**

---

## 2. Core Concepts

### 2.1 Symbol biography (the headline read surface)

A **symbol biography** is the rendered narrative attached to a knowledge-graph node — the chronologically-ordered, edge-typed sequence of tasks that touched the symbol, the decisions that governed it, and the rationale extracted from each task's record. This is the read surface that operationalizes the "oral history for agents" vision: an agent loading a symbol gets its story in one read, not as a graph dump but as readable prose ([2_design.md §6](./2_design.md)).

Biographies are produced by a **deterministic, templated renderer** in Phase 1 — no LLM summarization, because a renderer that paraphrases prior tasks unfaithfully is worse than no renderer. The agent reads the rendered biography and decides what's relevant. The renderer's job is faithful surfacing, not synthesis.

### 2.2 Task as a first-class graph node

A task record (`T...`) becomes a node in a graph parallel to the code knowledge graph. It carries the existing task fields (`description`, `plan`, `execution_summary`, `context_files`, `external_refs`, `status` history) plus a derived `edges` view that the lineage system populates and refreshes incrementally.

### 2.3 Typed edges

Edges between tasks are **typed**, not flat. Each edge carries (`from`, `to`, `type`, `derivation_source`, `confidence`, `evidence`, `at`). Edge types form the system's causal vocabulary:

| Edge | Meaning | Typical derivation |
|------|---------|---------------------|
| `motivates` / `motivated_by` | A surfaced the need that produced B | Description text scan |
| `extends` | B builds on A's primitive | Commit-grep + co-touched symbols |
| `supersedes` / `superseded_by` | B replaces A | ADR Status line, description text |
| `reverts` / `reverted_by` | B undoes part of A | Diff overlap with reverse sign |
| `blocks` / `blocked_by` | Runtime dependency | Declared in task |
| `contradicts` | Semantic conflict; one is wrong | Description / review thread |
| `references` | Weak — mentioned the other | Description / plan / summary text |
| `co-touches` | Shared `context_files` overlap | Commit-grep, kg attribution |
| `co-touches-symbol` | Shared graph node | Knowledge graph attribution |
| `runtime-parent` / `runtime-child` | Pipeline-spawned | Activity/job run linkage |
| `cited-in-review` | Reviewer cross-citation | Review thread text scan |

Typed edges are the **grammar of the biography**. The renderer turns `B supersedes A` into "task B superseded task A's approach"; `B reverts A` into "task B undid part of task A's change"; `B extends A` into "task B built on the primitive added in task A." Without the typed vocabulary, the rendered biography flattens into "task B is related to task A," which is the failure mode of every hand-authored issue-tracker link graph.

### 2.4 Bipartite bridge to the code knowledge graph

Lineage is **not** a standalone task graph. It is the task half of a bipartite graph whose other half is the existing code knowledge graph, and the bipartite shape is what makes biographies keyable on symbols. Cross-plane edges:

- `task touched code-node` — derived from commit attribution.
- `task declared code-node` — from `context_files`.
- `task created code-node` / `task deleted code-node` — from diff side analysis.
- `task superseded ADR-bound-decision` — from ADR Status line resolution.

This is the structural piece nothing in prior art has — Jira/Linear don't own the code, and the knowledge graph alone doesn't own the decision/plan layer. Closure operations cross planes freely. "Render the biography of `Foo::bar()`" is one CTE that walks `task_code_edges` → `task_edges` → renderer; without the bipartite shape it would be three separate queries the consumer has to compose.

### 2.5 Derivation pipeline (minimal Phase 1)

Edges are produced by three Phase 1 derivers, each emitting `(edge, evidence, confidence)` tuples:

1. **commit-grep** — `git log --grep '\[T...\]' -- <path>` → `co-touches` edges across files in `context_files`, plus `touched` task-code edges.
2. **kg-attribution** — restored per [4_decisions.md ADR-001](./4_decisions.md) — node-level `task_ids` give symbol-grain `co-touches-symbol` edges plus `touched` at symbol grain.
3. **task-text** — regex `\[T\d{8}-\d+(?:-\d+)*\]` over task `description` / `plan` / `execution_summary` → typed `references` edges, upgraded to `supersedes` / `extends` / `reverts` / `motivates` when adjacent verbs match.

Every edge is timestamped and carries its derivation source. Re-running a deriver is idempotent: the edge store dedupes by `(from, to, type, source)`. Additional derivers (`adr-text`, `runtime-link`, `review-cite`) are deferred — see [2_design.md §2.1](./2_design.md).

### 2.6 Closure operations (minimal Phase 1)

The minimal Phase 1 surface is one named closure plus the rendered biography that consumes it:

- **`feature` closure** — given a code selector (file, symbol, KG `stable_id`), return all attributed tasks chronologically with the cross-edges among them. The structured substrate the renderer reads from.
- **`biography`** — same input as `feature`; output is the rendered prose narrative (§2.1) plus the structured payload alongside it. **The headline read surface of the minimal Phase 1.**

Additional closures (`closure`, `decision`, `reversal`, `risk`) are deferred — see [2_design.md §5](./2_design.md). A raw `edges` escape hatch is provided for ad-hoc traversals before those land.

### 2.7 Consumers are deferred

The minimal Phase 1 ships only the substrate (derivation + bipartite bridge + storage) and the read surface (`feature` closure + biography renderer). It deliberately *does not* ship consumers:

- Authoring assist in `orbit-create-task` (recursive `context_files` expansion through lineage closure) — deferred.
- Stale-task detection sweep — deferred.
- ADR auto-supersession suggestion — deferred.
- Auto-fire-on-PR "review assist" hook — explicitly removed; on-demand closure calls cover the reviewer-initiated cases.

The first draft proves the substrate. Consumers land in follow-up phases once the substrate is shipped, instrumented, and showing real biography traffic.

### 2.8 Edge store + provenance

Edges persist in a SQLite table alongside the task store. Schema sketch:

```sql
CREATE TABLE task_edges (
  from_task_id TEXT NOT NULL,
  to_task_id   TEXT NOT NULL,
  edge_type    TEXT NOT NULL,
  source       TEXT NOT NULL,   -- commit-grep | kg-attribution | adr-text | ...
  confidence   REAL NOT NULL,   -- 1.0 declared, derived edges score by signal
  evidence     TEXT NOT NULL,   -- JSON: commit SHA, ADR section, line range, etc.
  at           TIMESTAMP NOT NULL,
  PRIMARY KEY (from_task_id, to_task_id, edge_type, source)
);
```

Workspace-scoped per the [CLAUDE.md](../../../CLAUDE.md) Scoping Rules table. Provenance is non-negotiable: every edge is auditable, every line of the rendered biography traces back to a source row, and a flagged-incorrect edge becomes feedback for the deriver, not just a one-off correction.

### 2.9 Temporal slicing

Every edge is timestamped, every task carries status history with timestamps, every commit has an author date. Lineage queries accept an `as-of` parameter, so an agent can ask "what would the biography have shown when T was created?" — useful for postmortem analysis and for staying honest when reading old execution summaries that cite tasks superseded since.

### 2.10 Symbol identity stability is the load-bearing assumption

Biographies survive structural change *only* if KG `stable_id` survives the structural change. If a refactor that splits one function into three reallocates `stable_id`s, every biography keyed on the old IDs is orphaned. The design assumes KG identity is stable across renames, file moves, and most refactors; the boundary cases (extract-into-multiple, inline-into-caller) are open and named in [2_design.md §7](./2_design.md). This assumption is the most fragile point of the whole feature, and it is named explicitly so reviewers can challenge it.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Edge schema (minimal type set), derivation pipeline (3 derivers), storage layout, bipartite bridge | [2_design.md §1–§4](./2_design.md) | — |
| Closure tool surface (`feature` + `biography` + `edges` + `rebuild`) | [2_design.md §5](./2_design.md) | — |
| **Symbol-biography renderer (Phase 1 headline read surface)** | [2_design.md §6](./2_design.md), [4_decisions.md ADR-008, ADR-009](./4_decisions.md) | — |
| **Symbol identity stability across refactors (load-bearing assumption)** | [2_design.md §7](./2_design.md), [4_decisions.md ADR-010](./4_decisions.md) | — |
| **Open concerns to address one by one** | [2_design.md §9](./2_design.md) | — |
| Restoration of code-graph task attribution | [4_decisions.md ADR-001](./4_decisions.md) supersedes [knowledge-graph ADR-029](../knowledge-graph/4_decisions.md) | [T20260506-11] |
| Deferred: authoring assist, stale-task detection, ADR auto-supersession, additional derivers/closures, cross-machine rendering | [3_vision.md](./3_vision.md) | — |
| Open questions, prior art, ambitious projections | [3_vision.md](./3_vision.md) | — |
| Glossary | [references/glossary.md](./references/glossary.md) | — |

---

## Task References

- [T20260506-11] — Removed graph task attribution; superseded by this feature's [ADR-0121](./4_decisions.md).
- [T20260506-15] — Added `orbit.task.locks.reserve` `files` shape and the `orbit-locks` skill; the originating-task lineage exemplar that motivates derivation-first edges.
- [T20260510-17] — Removed agent-facing lock instructions; the authoring failure that surfaced lineage as a need.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
