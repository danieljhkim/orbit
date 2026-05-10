# Task Lineage — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-09

This is the **minimal first-draft design**. It specifies only the primitives required to make the "oral history for agents" vision in [1_overview.md §1](./1_overview.md) real: the edge schema, three derivers, the bipartite task↔code bridge, the `feature` closure substrate, the symbol-biography renderer, and the load-bearing assumption that KG `stable_id` survives structural change.

Everything else — additional derivers (`adr-text`, `runtime-link`, `review-cite`), additional closures (`closure`, `decision`, `reversal`, `risk`), authoring assist, stale-task detection, ADR auto-supersession suggestion, cross-machine rendering — is deliberately **deferred** to follow-up phases. The first draft ships only what the vision requires; consumers and polish wait until the substrate has shipped and proved load-bearing. Forward-looking sketches of those deferred pieces live in [3_vision.md](./3_vision.md).

The symbol biography is the read surface lineage exists *for*. Edge schema, derivers, and storage are the substrate that makes the biography possible; the agent-facing payoff is "load this symbol, read its history in one shot."

---

## §1 Edge Schema

The lineage system models two edge planes:

1. **Task-task edges** in `task_edges` — one row per derived or declared edge between two task nodes.
2. **Task-code edges** in `task_code_edges` — one row per derived attribution between a task node and a code-graph node (file, symbol, section, config key).

Both tables share a common shape:

```sql
CREATE TABLE task_edges (
  from_task_id  TEXT NOT NULL,
  to_task_id    TEXT NOT NULL,
  edge_type     TEXT NOT NULL,
  source        TEXT NOT NULL,
  confidence    REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
  evidence      TEXT NOT NULL,           -- JSON; shape varies per source
  derived_at    TIMESTAMP NOT NULL,
  workspace     TEXT NOT NULL,           -- workspace_orbit_dir
  PRIMARY KEY (workspace, from_task_id, to_task_id, edge_type, source)
);

CREATE INDEX idx_task_edges_to    ON task_edges(workspace, to_task_id, edge_type);
CREATE INDEX idx_task_edges_from  ON task_edges(workspace, from_task_id, edge_type);

CREATE TABLE task_code_edges (
  task_id          TEXT NOT NULL,
  node_stable_id   TEXT NOT NULL,        -- KG stable_id (`node:<nanoid-21>`)
  node_kind        TEXT NOT NULL,        -- file | symbol | section | config_key
  edge_type        TEXT NOT NULL,        -- touched | declared | created | deleted
  source           TEXT NOT NULL,
  confidence       REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
  evidence         TEXT NOT NULL,
  derived_at       TIMESTAMP NOT NULL,
  workspace        TEXT NOT NULL,
  PRIMARY KEY (workspace, task_id, node_stable_id, edge_type, source)
);

CREATE INDEX idx_task_code_edges_node ON task_code_edges(workspace, node_stable_id, edge_type);
CREATE INDEX idx_task_code_edges_task ON task_code_edges(workspace, task_id, edge_type);
```

### 1.1 Edge type contracts (Phase 1 minimal set)

Task-task edge types:

| `edge_type` | Direction | Contract |
|-------------|-----------|----------|
| `supersedes` / `superseded_by` | A→B / B→A | B replaces A. |
| `extends` | A→B (B extends A) | B builds on a primitive added in A. |
| `reverts` / `reverted_by` | A→B / B→A | B undoes part of A's diff. |
| `motivates` / `motivated_by` | A→B / B→A | A surfaced the need that produced B; one direction stored, the inverse derived on read. |
| `references` | A→B | Weakest edge. A's text mentions B without a stronger verb. The default when text-derivation finds a `[T...]` mention without a verb upgrade. |
| `co-touches` | A↔B (symmetric) | A and B share at least one file in `context_files` (or commit attribution). |
| `co-touches-symbol` | A↔B (symmetric) | A and B share at least one knowledge-graph node. Stronger than `co-touches`. |

Task-code edge types:

| `edge_type` | Contract |
|-------------|----------|
| `touched` | Task's commits modified the node. |
| `declared` | Task's `context_files` declared the node. |
| `created` | Task's commits added the node (didn't exist at task start). |
| `deleted` | Task's commits removed the node. |

**Edge types deliberately deferred from this minimum.** `blocks` / `blocked_by` (waits on declared-dependency integration), `contradicts` (rare, high-noise, and would require a deriver this minimum doesn't ship), `runtime-parent` / `runtime-child` (waits on a `runtime-link` deriver), `cited-in-review` (waits on a `review-cite` deriver), `governed_by_adr` (waits on an `adr-text` deriver). The `edge_type` column is free-form TEXT, so any of these can be added later without a schema migration; the contract above documents what is *populated* by Phase 1, not what is *allowed* by the schema.

### 1.2 Evidence schema (per source)

Evidence is JSON, with a per-source schema. Phase 1 ships three sources:

```json
// commit-grep
{ "commit_sha": "d4f320f9", "path": "crates/.../skill.rs", "lines_changed": 14 }

// kg-attribution
{ "node_stable_id": "node:abc...", "build_ref": "agent-main@<oid>", "extractor": "rust" }

// task-text
{ "task_field": "description", "match_line": "...supersedes [T20260506-15]...",
  "verb": "supersedes", "char_offset": 412 }
```

Provenance is non-negotiable. Every edge can be opened to its original source row, range, or commit. Wrong derivations are flagged for the *deriver*, not corrected as one-offs.

---

## §2 Derivation Pipeline

A deriver is a Rust trait:

```rust
pub trait EdgeDeriver: Send + Sync {
    fn name(&self) -> &'static str;          // matches `source` in the edge table
    fn run(&self, ctx: &DeriveCtx) -> Result<DeriveDelta, OrbitError>;
}

pub struct DeriveCtx<'a> {
    pub workspace_orbit_dir: &'a str,
    pub task_store: &'a TaskStore,
    pub kg: &'a KnowledgeGraph,
    pub run_store: &'a RunStore,
    pub since: Option<DerivationCursor>, // last successful run watermark
}

pub struct DeriveDelta {
    pub edges_upserted: Vec<Edge>,
    pub task_code_edges_upserted: Vec<TaskCodeEdge>,
    pub cursor: DerivationCursor,
}
```

### 2.1 The three Phase 1 derivers

| Source | Reads | Emits | Cursor |
|--------|-------|-------|--------|
| `commit-grep` | git log on each task's `context_files` paths + `[T...]` regex over messages | `co-touches` (task↔task), `touched` (task↔code-file) | latest indexed commit OID |
| `kg-attribution` | knowledge-graph node `task_ids` field (restored per [4_decisions.md ADR-001](./4_decisions.md)) | `co-touches-symbol`, `touched` (task↔code-symbol) | KG ref OID |
| `task-text` | task `description`, `plan`, `execution_summary` fields | `references` (default), upgraded to `supersedes` / `extends` / `reverts` / `motivates` when adjacent verbs match | task `updated_at` |

Run order is independent — derivers are pure functions of their inputs and never read each other's output. The pipeline runs them in parallel and merges deltas at commit time.

**Derivers deferred from this minimum.** `adr-text` (ADR Status-line scanning), `runtime-link` (activity/job parent/child), `review-cite` (review-thread citations). Each is straightforward to add when its consumer ships; none of the Phase 1 minimum needs them.

### 2.2 Verb upgrade rules (`task-text` deriver)

The `task-text` deriver scans for `\[T\d{8}-\d+(?:-\d+)*\]` and looks at a ±60-character window around the match. If a verb in the upgrade table fires, the edge is emitted at the upgraded type instead of `references`:

| Window phrase (case-insensitive regex) | Edge type |
|----------------------------------------|-----------|
| `superseded? by`, `supersedes?` | `supersedes` |
| `reverts?`, `reverting`, `undoes?` | `reverts` |
| `extends?`, `builds on`, `follow-?up to` | `extends` |
| `motivated by`, `surfaced by`, `triggered by` | `motivates` |

No verb match → `references` (confidence 0.4). Verb match → upgraded edge type (confidence 0.8). Multiple verb matches in the same window → emit each as a separate edge (the rare case where authors say "supersedes and extends").

**Excluded from text-derivation by design.** Two edge types in §1.1 are deliberately *not* produced by this deriver:

- `blocks` / `blocked_by` — has a stronger source: declared task `dependencies`. Text-derivation would add noise to a field that already has a canonical source.
- `contradicts` — base rate is too low and false-positive risk is too high. The verb "conflicts with" fires on benign phrases ("doesn't conflict with") in the ±60-char window. Edge type stays in the schema for declared use; text-derivation is out of scope.

Narrowing to four upgrade rules is a Phase 1 scoping choice. The four chosen rules — `supersedes`, `extends`, `reverts`, `motivates` — are the verbs of the §6 biography renderer's narrative grammar; deriving them well is what makes biographies readable.

### 2.3 Confidence calibration

| Source | Default confidence | Upper bound |
|--------|---------------------|-------------|
| Declared (human or agent set the field) | 1.0 | 1.0 |
| `commit-grep` | 0.95 (commit message convention is enforced) | 1.0 |
| `kg-attribution` | 0.9 (build can drift) | 1.0 |
| `task-text` (upgraded) | 0.8 | 0.9 |
| `task-text` (`references` default) | 0.4 | 0.6 |

Closure operations accept a `min_confidence` parameter (default 0.5) so consumers can dial in noise tolerance. The biography renderer defaults to 0.7 so rendered narratives lean toward fewer-but-truer claims. See [3_vision.md §1.1](./3_vision.md) for confidence-calibration questions left open.

### 2.4 Incremental rebuild

Each deriver persists a cursor in a `derivation_cursors` table. On each run, the deriver reads only inputs newer than its cursor and upserts edges. The pipeline is therefore O(Δ) on inputs, not O(N) on history.

Triggers that schedule a deriver run:

- **commit hook (`post-commit`)** → `commit-grep`, `task-text`.
- **graph rebuild (`orbit graph build`)** → `kg-attribution`.
- **task store mutation** → `task-text`.

A `orbit lineage rebuild --force` CLI subcommand drops cursors and re-derives from scratch; it is the only path that costs O(N) and is the recovery surface for broken deriver state.

---

## §3 Storage Layout & Scoping

| Artifact | Storage | Scope | Rationale |
|----------|---------|-------|-----------|
| `task_edges`, `task_code_edges`, `derivation_cursors` | SQLite alongside the existing task store DB | WorkspaceOnly | Edge table is a derived index; it lives where the task store lives, in `.orbit/state/`. |
| Edge JSON evidence | inline in the `evidence` column | WorkspaceOnly | Evidence is small per-row; a sidecar would add no value. |

The scoping table in [CLAUDE.md](../../../CLAUDE.md) gets one new row added when this ships:

```
| Lineage Edges  | WorkspaceOnly      | Derived from local tasks/commits/runs            |
```

Schema migrations follow the existing orbit-store layered pattern. Initial migration adds the three tables. Backwards-compat: the migration runs on an existing workspace with no edges; a follow-up `orbit lineage rebuild` populates them.

---

## §4 Bipartite Bridge: Task ↔ Code Graph

The lineage system is **not** a separate graph that happens to reference code-graph nodes. It is one half of a single bipartite graph whose other half is the code knowledge graph. This shapes two things:

### 4.1 Stable IDs across planes

Code-graph nodes already carry `stable_id: node:<nanoid-21>` per [knowledge-graph ADR-010](../knowledge-graph/4_decisions.md). Task nodes use their existing `task_id`. The two ID namespaces don't overlap (different prefix, different shape), so a single `node_id` column with a kind discriminator works in any traversal query.

### 4.2 Cross-plane closure queries

Cross-plane closures are recursive CTEs that union across the two edge tables:

```sql
WITH RECURSIVE lineage(task_id, depth) AS (
  SELECT :seed_task, 0
  UNION
  SELECT te.to_task_id, l.depth + 1
    FROM task_edges te
    JOIN lineage l ON te.from_task_id = l.task_id
   WHERE te.workspace = :ws
     AND te.confidence >= :min_conf
     AND l.depth < :max_depth
)
SELECT DISTINCT task_id FROM lineage;
```

The "what code did this lineage touch?" query joins the result onto `task_code_edges`. The "what tasks touched this symbol?" query starts from `task_code_edges` and walks back into `task_edges`. A single SQL surface, two reasoning directions.

### 4.3 KG attribution restoration

Phase 1 requires reviving the symbol-grain `task_ids` attribution removed by [knowledge-graph ADR-029](../knowledge-graph/4_decisions.md). The new attribution is shaped narrowly for lineage's needs:

- **What's persisted on graph nodes:** `task_ids: Vec<String>` (set, not ordered list); no per-hunk attribution; no historical task_ids beyond what current commits attribute.
- **What's not persisted:** the old hunk-mapping walker, the `structural_conflict` field, the task-commits sidecar.
- **Build cost:** restored attribution is a single pass over commits in the build window, executed alongside the existing extraction pass. Empirically (per the audit cited in ADR-029), the dominant cost was hunk-coordinate remapping after rename hops; we drop that.

[4_decisions.md ADR-001](./4_decisions.md) is the supersession record.

---

## §5 Closure Tool Surface

Closures are the agent-facing API. The minimal Phase 1 surface is two named closures plus a raw escape hatch and a rebuild command:

```
orbit.lineage.feature       — given a code selector, return all attributed tasks (substrate for §6)
orbit.lineage.biography     — given a code selector, return the rendered biography (§6)
orbit.lineage.edges         — raw enumeration (escape hatch)
orbit.lineage.rebuild       — CLI-only; force re-derivation
```

**Closures deferred from this minimum.** `closure` (task-seed lineage walk; waits on authoring-assist consumer), `decision` (waits on `adr-text` deriver), `reversal` (waits on its consumers), `risk` (waits on authoring-assist). The `edges` escape hatch covers the cases an agent needs to compose ad-hoc traversals before those land.

### 5.1 `orbit.lineage.feature` (biography substrate)

Inputs accept a code selector (`file:`, `dir:`, `symbol:`, `node:<stable_id>`). Output is a chronologically-ordered task list with `edge_type` ∈ {`touched`, `declared`, `created`, `deleted`}, plus the union of cross-edges (`supersedes`, `extends`, `reverts`, `motivates`) among the returned tasks. This is the structured input the §6 renderer consumes; it is also directly usable by agents that want to build their own narrative.

### 5.2 `orbit.lineage.biography` (rendered read surface)

Same input shape as `feature`. Output is a rendered prose biography (§6) plus the structured `feature` payload alongside it for agents that want both. **This is the headline tool of Phase 1.** Defaults are tuned for "load a symbol, get its story": `min_confidence=0.7`, all populated edge types included, no temporal cutoff.

---

## §6 Symbol Biography Renderer (Phase 1 headline)

The renderer is the read surface that operationalizes "oral history for agents." It takes the structured output of `orbit.lineage.feature` and produces a deterministic, templated narrative the agent reads in one shot.

### 6.1 Why the renderer exists

A graph dump is not oral history. An agent that loads a symbol's biography should see something like:

> **`orbit_tools::skill::lock_reservation` — biography**
>
> Created in [T20260420-08] as part of the original lock-reservation skill (the `paths` shape), motivated by sandbox policy needing a single arbitration point. Status: done.
>
> Extended in [T20260506-15] (the `files` shape) — extends [T20260420-08]'s primitive to allow direct file reservations without re-resolving paths. Status: done. ADR-019 (knowledge-graph) was flipped to Accepted at this point.
>
> **Removed in [T20260510-17]** — supersedes both prior tasks. The agent-facing skill instructions were dropped because lock arbitration moved into the runtime. Status: done. Note: this task's execution summary acknowledged that [README.md:27](../../../README.md) sales copy still describes the removed feature (still-live as of biography render).
>
> Cross-references: [knowledge-graph ADR-029](../knowledge-graph/4_decisions.md) governs this symbol's domain; superseded by lineage [4_decisions.md ADR-001](./4_decisions.md).

That output is what an agent loading the symbol gets. Each line traces back to a specific edge in the substrate; the agent can audit every claim.

### 6.2 Rendering rules

The Phase 1 renderer is **deterministic and templated** — no LLM summarization. ADR-009 names the cost. The rules:

1. **Group tasks by `edge_type` over time.** Created → extended → reverted → superseded → removed, in chronological order.
2. **Render each task as one paragraph** of the form: `<verb-phrase from edge_type>` in `[T...]` (`<task summary>`). `<one-sentence pull from task description, plan, or execution_summary, picked by template>`. Status: `<status>`.
3. **Render cross-edges (`supersedes`, `extends`, `reverts`, `motivated_by`) inline as the verb phrase**, not as a separate "edges" list. The grammar of the biography *is* the edge taxonomy.
4. **Attach ADR citations inline** when an ADR's domain covers the symbol — pulled from `task_code_edges` with `edge_type = governed_by_adr`.
5. **Surface still-live risks** — execution-summary excerpts from any task whose `reversal` closure includes the current state. Marked with bold "Note:".
6. **No interpretation, no synthesis.** The renderer pulls strings from canonical fields and arranges them per the template. If a task's record is terse, the rendered paragraph is terse. The renderer does not invent rationale.
7. **Truncation rule.** Default budget: 2000 tokens. The budget is calibrated to current agent context-window economics — a biography that costs more than ~2000 tokens crowds out the rest of the agent's working set. As frontier-model context windows grow and per-token attention quality at depth improves, this budget rises; the constant is a tunable, not a design ceiling. If the substrate exceeds the budget, the renderer drops oldest `co-touches` first, then oldest `references`, then oldest tasks by chronological order, surfacing a `truncated: true` flag and a `dropped_count` so the agent can request the full payload via `orbit.lineage.feature`.

### 6.3 Why deterministic in Phase 1

An LLM-summarizer would be more readable but produces output the agent cannot audit cheaply. A renderer that paraphrases prior tasks unfaithfully introduces *false oral history* — the failure mode that destroys agent trust in the surface. Phase 1 ships with the deterministic renderer; Phase 3 may add an optional LLM-summarized layer behind a feature flag, but only after the deterministic renderer has shipped and proved the substrate carries weight. ADR-009 holds the line.

### 6.4 The `task-text` deriver is the renderer's narrative bottleneck

The `task-text` deriver (§2.2) produces the most narrative-bearing edges (`supersedes`, `extends`, `reverts`, `motivates`). Wrong upgrades from this deriver become *wrong sentences in the biography* — not a missing edge but a falsified verb. Phase 1 ships this deriver only after a hand-labeled eval set of ≥100 task-text edges is run through it with measured precision/recall. The eval gate is named explicitly in §9.

### 6.5 Where the renderer lives in the stack

The renderer is a function of the structured `feature` payload. It lives in `orbit-tools::lineage::biography` and is exposed to agents as `orbit.lineage.biography`. CLI surface: `orbit lineage show <selector>` for human / debugging use, identical output. The biography is computed on demand, not pre-rendered — the substrate is dense enough for sub-second rendering at expected workspace scales.

---

## §7 Symbol Identity Stability (Load-Bearing Assumption)

The biography surface (§6) keys every story on a KG `stable_id`. If `stable_id` is unstable across the structural changes lineage is meant to survive, biographies orphan and the entire vision collapses. This section names the assumption explicitly so reviewers can challenge it, and names the cases where it is known to break.

### 7.1 What KG stable_id guarantees today

Per [knowledge-graph ADR-010](../knowledge-graph/4_decisions.md), `stable_id: node:<nanoid-21>` is allocated on a node's first appearance and intended to survive renames and file moves. Concretely:

- **File rename** (`a.rs` → `b.rs`): file-node `stable_id` survives; child-symbol `stable_id`s survive.
- **Symbol rename within file** (`fn foo` → `fn bar`): symbol `stable_id` survives.
- **Function moved to a different file** (`a.rs::foo` → `b.rs::foo`): symbol `stable_id` survives if KG's identity tracker can rejoin the symbol across files.

### 7.2 Where the assumption breaks

The cases where `stable_id` is known *not* to survive:

- **Extract-into-multiple** (one function split into three): one of the three may keep the original `stable_id` (heuristic-dependent); the other two get fresh IDs. The biography on the original `stable_id` will follow only one of the three; the other two start blank.
- **Inline-into-caller**: the inlined symbol's `stable_id` is destroyed. Its biography is orphaned. The caller's biography does not absorb it.
- **Cross-language ports** (e.g., a Rust function ported to Go): no shared identity at all. Biography does not transfer.
- **Wholesale rewrite** of a file: identity tracker may or may not rejoin, depending on similarity heuristics. Boundary case.

These are real failure modes. Phase 1 ships with these limitations and surfaces them in the biography rendering: when a symbol's biography starts more recently than its surrounding file's biography, the renderer emits a "(biography may be incomplete — symbol identity changed at <commit>)" footnote.

### 7.3 Why we ship anyway

The cases where stability *does* hold cover the dominant refactor patterns in long-lived code (renames, file moves, single-function moves). The orphan cases are real but not the common case. The honest cost is named in [4_decisions.md ADR-010](./4_decisions.md): biographies will be wrong in some boundary cases, and the renderer's footnote is the user-facing acknowledgment. If the orphan rate proves high in practice, the response is to invest in KG identity-tracking heuristics (a knowledge-graph problem), not to weaken the biography surface.

### 7.4 Mitigations on the lineage side

- **Cross-rename evidence accumulation.** When the `commit-grep` deriver sees a rename in commit history, it records both old-path and new-path attribution under the surviving `stable_id`. This recovers a portion of the rename cases the KG identity tracker misses.
- **Path-grain fallback.** When a symbol-grain biography is empty but the containing file-grain biography is rich, the renderer falls back to file-grain history for that symbol with a "(symbol-grain history unavailable; showing containing-file history)" annotation.
- **Manual identity stitch.** The `orbit.lineage.edges` escape hatch supports a `declared` `same_as` edge between two `stable_id`s for the case where a human knows two IDs refer to the conceptually-same symbol.

---

## §8 Concerns & Honest Limitations

1. **Confidence is heuristic.** The numbers in §2.3 are starting points. Real calibration requires a labeled dataset of edge correctness, which doesn't exist yet. Phase 1 ships with the heuristic numbers; calibration is a follow-up.

2. **KG attribution restoration costs.** [Knowledge-graph ADR-029](../knowledge-graph/4_decisions.md) removed attribution citing zero reverse-lookup uses. We're restoring it for one consumer (the biography surface). If Phase 1's biography surface doesn't drive enough adoption to justify the cost, the right move is to roll back attribution again — not to invent more consumers to justify a sunk cost. [4_decisions.md ADR-001](./4_decisions.md) names the kill criterion.

3. **Edge-type taxonomy is intentionally narrow.** Phase 1 populates 7 task-task and 4 task-code edge types (§1.1). Adding more later is a schema-tolerant change (the column is free-form TEXT) but every consumer that ships against the smaller set will need to be updated when new types arrive. Cost: future consumers may need updates rather than naturally absorbing new types.

4. **The bipartite bridge means lineage's correctness is partly the graph's correctness.** A KG attribution bug shows up as wrong lineage edges. This is acceptable because the graph is the authoritative substrate, but it does mean lineage inherits the graph's failure modes. ([Knowledge-graph 2_design.md](../knowledge-graph/2_design.md) covers those.)

5. **Lineage cannot resurrect lost rationale.** If a task's `description` and `execution_summary` were terse, the biography surfaces them but they don't say much. Lineage amplifies the existing record's value; it does not generate retrospective rationale where none was written. The cure is task-authoring quality, not lineage.

6. **Workspace-locality is a real constraint.** Two engineers working on the same feature on different machines have disjoint local task graphs. Cross-machine biography composition is out of scope for the minimal Phase 1; it waits on follow-up work named in [3_vision.md §1.9](./3_vision.md).

---

## §9 Open Concerns to Address

The §8 list above is *limitations we ship knowing about*. This section is *open work items the minimal Phase 1 has not yet resolved* — concerns that need a concrete decision before the affected mechanism ships. Each carries a proposed resolution shape; the actual resolution lands as a follow-up task.

### 9.1 `task-text` deriver eval set (blocks Phase 1 ship) — RESOLVED to four rules

The `task-text` deriver (§2.2) produces the most narrative-bearing edges and is the renderer's primary source of narrative verbs. A wrong upgrade becomes a *false sentence in the biography* — not a missing edge, an actively-wrong claim. Confidence band 0.8 is a guess.

**Scope decision (resolved 2026-05).** The deriver derives only four upgrade rules: `supersedes`, `extends`, `reverts`, `motivates`. `blocks` is left to declared task `dependencies` (canonical source); `contradicts` is excluded as too high-noise for text derivation. See §2.2 "Excluded from text-derivation by design." This narrows the eval scope correspondingly.

**Resolution shape (eval set).** Hand-label ≥100 candidate edges from the deriver against the live task store, distributed across the four rules. Measure precision and recall per rule. If precision <0.85 for any rule, the deriver runs that rule in observation-only mode (writes edges with confidence ≤0.5, excluded from the renderer's default `min_confidence=0.7`) until it tightens. Ship-blocking for §6.

### 9.2 Renderer truncation strategy validation (blocks Phase 1 ship) — RESOLVED on budget intent

The §6.2 truncation rule (2000-token budget, drop oldest `co-touches` first) is calibrated to current agent context-window economics. A workspace with a 3-year-old feature could have biographies that overflow the budget routinely; the wrong drop-order would silently strip the most useful history.

**Budget-intent decision (resolved 2026-05).** Keep the budget tight for now (~2000 tokens) — the biography must fit alongside the rest of the agent's working set, and current models pay attention quality at depth. As frontier-model context windows grow and long-context attention improves, the budget rises in lockstep. The constant is tunable, not a design ceiling. The renderer reads the budget from runtime config so a future bump is a config change, not a code change.

**Resolution shape (drop-order validation).** Render biographies for 20 representative symbols spanning the codebase's age distribution. Measure: how often truncation fires at the current budget, what gets dropped, whether the rendered prefix preserves the load-bearing history. Tune drop-order based on results. Ship-blocking for §6 (drop-order tuning); not ship-blocking for the budget value itself.

### 9.3 Storage growth ceiling

Three derivers × every commit on a long-lived workspace lands in thousands-to-tens-of-thousands of edges. The doc says "millisecond latency at expected workspace scales" without naming the scale.

**Resolution shape.** Estimate steady-state row count for a 1-year-old workspace by extrapolating from current task/commit volume. If projected count exceeds a threshold (TBD; first guess 100k edges per workspace), specify a pruning policy: confidence-decay over time, or hard age-based pruning of low-confidence `co-touches`/`references` edges. Not ship-blocking for the minimal Phase 1, but must be resolved before the design is `Accepted`.

### 9.4 Symbol identity orphan rate measurement

§7 names the failure modes where `stable_id` does not survive structural change. The orphan rate in practice is unknown.

**Resolution shape.** Instrument the §6.2 footnote ("biography may be incomplete — symbol identity changed at <commit>") to record metrics. After 30 days of biography traffic, measure the orphan rate. If >10% of biography renders carry the footnote, escalate to KG identity-tracking heuristics work as a follow-up task. Not ship-blocking for Phase 1.

---

## Task References

- [T20260506-11] — Removed graph task attribution; superseded by [4_decisions.md ADR-001](./4_decisions.md).
- [T20260506-15] — Added `orbit.task.locks.reserve` `files` shape and the `orbit-locks` skill; canonical lineage exemplar.
- [T20260510-17] — Removed agent-facing lock instructions; the authoring failure that surfaced lineage.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
