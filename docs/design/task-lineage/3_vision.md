# Task Lineage — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-09

This document is forward-looking. It names the open questions Phase 1 deliberately leaves unanswered, surveys the prior art that shapes our reading of "novelty", and articulates the projection of lineage we believe is genuinely new. It is not a roadmap; it is the field-of-view inside which roadmap tasks will be authored.

The shape of the document: §1 enumerates open questions ordered by load-bearing-ness. §2 surveys prior work in eight categories. §3 names what may be distinctive about Orbit's take and *why*. §4 gathers Orbit-internal and external references. §5 lists task IDs cited.

---

## §1 Open Questions

### 1.1 How do we calibrate edge confidence?

The numbers in [2_design.md §2.3](./2_design.md) are heuristics chosen by inspection. Real calibration requires a labeled corpus: pairs of `(edge, source, was_actually_relevant)` judged by humans or downstream verification. Three options:

- **Implicit feedback.** When an authoring agent dismisses a suggested lineage task at expansion time, treat that as a negative signal on the edge. Cheap to collect, biased by agent skill.
- **Explicit feedback.** A `lineage feedback` tool that lets an agent or human flag an edge as right/wrong, feeding a per-source calibration.
- **Outcome-driven.** Track whether the *task* expanded a `context_files` based on a lineage suggestion, and whether that file was actually modified by the time the task hits review. The strongest signal but requires post-hoc joining.

Phase 1 ships with implicit + explicit; outcome-driven is Phase 2 and waits on enough volume to be statistically meaningful.

### 1.2 When is recursive expansion's fixed point the wrong stopping condition?

Lineage closure expands until no new tasks appear. For features whose history spans a year and dozens of tasks, fixed point may produce a closure of 40+ tasks — useful for archaeology, useless for authoring assist. Open: should the authoring loop terminate at a *different* condition than the archaeology loop? Candidates:

- **Recency cutoff.** Drop tasks older than N days from authoring closure.
- **Diminishing-returns cutoff.** Stop expanding when each new round contributes fewer than K new files.
- **Confidence-weighted depth.** Cap depth at the deepest level whose minimum confidence stays above a threshold.

Probably some combination; this is a UX problem masquerading as a graph problem.

### 1.3 How do we evolve the edge-type taxonomy after rollout?

[2_design.md §1.1](./2_design.md) ships 12 edge types. New types need to be addable without a Big Migration. Open:

- Should the taxonomy be a string-typed open enum, with consumers required to handle unknown types gracefully?
- Or a closed enum, with type additions as schema migrations?

The closed-enum tradeoff is type safety in Rust at the cost of every consumer having to be updated together. The open-enum tradeoff is flexibility at the cost of consumers needing tolerant code paths. Knowledge-graph chose closed enum + careful versioning ([knowledge-graph ADR-011](../knowledge-graph/4_decisions.md)); lineage may want the same constraint.

### 1.4 Can lineage edges become bidirectional rationale storage?

A `supersedes` edge has obvious forward semantics ("B replaces A"). But it implicitly carries reverse rationale: "A is the prior approach we considered and chose against B until we didn't." Today that rationale lives only in B's description. Open: should the edge itself store a `rationale` field, populated at edge creation time, that becomes searchable as a first-class artifact?

The risk: rationale duplicates the source task's content and rots independently. The opportunity: a rationale-rich edge graph is queryable as "show me all the times we walked back from approach X" — a kind of organizational learning surface that no current Orbit artifact provides.

### 1.5 How does lineage interact with `learning` records?

[Project-learnings](../project-learnings/) is a parallel system: rules and gotchas surfaced into agent context. Tasks and learnings overlap conceptually — a learning often *describes* a recurring lesson encoded across multiple task lineages. Open:

- Should `learning` records be a third node type in the graph, with edges to the tasks that produced them?
- Or should learnings be derived *from* lineage on demand (stale-task patterns, recurring `reverts` edges) rather than authored separately?

Most likely both: authored learnings as canonical, derived patterns as candidates the human curates into authored learnings.

### 1.6 What does cross-graph traversal mean for sandbox / policy decisions?

The bipartite bridge means an agent reasoning about a task can walk into code-graph nodes and from there into other tasks. That's a much wider reachable surface than today's task tools. Open: does the closure tool need a sandbox-aware filter that respects the activity's `fsProfile`? E.g., should an activity that can't read `crates/orbit-policy/` also be denied lineage hops *into* tasks attributed to that crate?

Probably yes — but the failure mode (an agent reasoning correctly about a feature it can't read) may be acceptable. The decision waits for [policy-sandbox](../policy-sandbox/) to weigh in.

### 1.7 Could lineage drive automatic decision archaeology?

Given a code symbol, lineage already produces "every task that touched it". Open: can we go further and reconstruct the *narrative* — render the sequence as a timeline of decisions, citing ADRs at each fork, with the task plans/summaries as commentary? This is the most ambitious read direction and is the core of Phase 3 (see §3).

The hard problem isn't the data; the data is in the bipartite graph. The hard problem is summarization that doesn't lie. A summarizer that paraphrases prior tasks badly is worse than no summarizer.

### 1.8 What's the right surface for "compare two lineage closures"?

When superseding a feature (e.g., the original lock-skill story), it's useful to compare the closure of the introducing task with the closure of the proposed superseding task: identify what one covers that the other doesn't. Open: should this be a first-class tool (`orbit.lineage.diff`)? Or a derived consumer of two `closure` calls?

Phase 1 doesn't ship this. The shape is small enough to add later if a consumer asks.

### 1.9 Cross-machine lineage — when does the local-only constraint break?

[2_design.md §9](./2_design.md) keeps lineage strictly local. The rendering layer surfaces external_refs for cross-engineer presentation. Open: are there workflows where two engineers' local lineage chains genuinely need to merge — not just be presented side-by-side, but be queryable as one graph?

Likely candidates: a shared production-incident root cause analysis where the relevant tasks live across multiple engineers' machines. The proposed answer is to *publish* lineage subgraphs to a shared store on demand (an export/import surface) rather than to make the live graph distributed. But the shape of the publish surface is open.

### 1.10 Can lineage edges be derived from non-Orbit signal?

Today the deriver list is six sources, all internal to Orbit. Open: a 7th deriver — `external-tracker` — that reads Linear/Jira issue links and maps them onto task-task edges via the external_refs spine. Useful when external trackers carry richer link graphs than the local task store does.

Phase 1 doesn't ship it; it depends on Linear/Jira tooling that's not yet present in Orbit's MCP surface.

---

## §2 Prior Work

### §2.1 Issue tracker linking

Jira, Linear, Asana, Shortcut, GitHub Issues, ClickUp, Azure DevOps. All support typed links between tickets — "blocks", "duplicates", "relates to", "is part of", "is caused by", "is fixed by". Linear adds parent/sub-issue and "tracks" relations. GitHub recently shipped sub-issues and "tracked in" cross-references.

What they share: typed edges, mostly hand-authored, optimized for human navigation in a sidebar. None expose the link graph as a programmatic substrate for context aggregation. None bind the link graph to the source code beyond surface-level "this PR closes that issue" mentions.

### §2.2 Requirements traceability

DOORS (IBM Rational), Polarion (Siemens), Jama Connect, IBM ELM. These are the heavyweight tools used in regulated industries (aerospace, medical devices, automotive) to maintain traceability from requirements through design through code through test. They are explicit graph databases over engineering artifacts, with audit-grade provenance.

What they share with our framing: typed edges, multi-artifact graph, queryable as closure. What they don't share: agent-readable surface, automatic derivation from commit/code signal, integration with a code knowledge graph. Their cost is enormous — typically requires a dedicated team and expensive tooling licenses.

### §2.3 ADR supersession

Architecture Decision Records, popularized by Michael Nygard's 2011 essay. Modern variants (MADR, Y-statements, Nygard form) all encode a `supersedes` / `superseded by` chain in the Status line. This is *exactly* the lineage edge for design decisions, executed at file granularity rather than at issue granularity.

Orbit's own ADR log already uses this pattern. Lineage's ADR-text deriver reads it as one of six signal sources. The piece prior art doesn't have: ADRs aren't bound to *tasks*, only to other ADRs. Lineage closes that loop.

### §2.4 Issue tracker mining (research)

Academic line of work that treats issue trackers as graphs and applies link prediction, deduplication, summarization, and bug-triage models to them. Notable strands:

- **Link recovery.** Inferring missing "duplicate" / "related-to" links via text similarity (Wang & Lo et al.).
- **Bug-fix attribution.** Mapping bug reports to fixing commits via commit-message scanning, file overlap, and stack-trace matching.
- **Knowledge graph construction over GitHub.** OSCAR, GitHub Knowledge Graph projects.

What they share with our framing: derivation from text + commit signal. What they don't: tight integration with a runtime task store, an authoring write-back loop, or symbol-level granularity.

### §2.5 Code provenance

`git blame`, `hg annotate`, Sourcegraph, GitHub blame UI. All answer "who last touched this line?" Some go further — `git log -L`, `git log --follow` — and walk a single line/symbol's history through renames. Sourcegraph's "code intelligence" extends this to symbol-level cross-repo references.

Lineage's `feature` closure is structurally similar to `git log -L` rolled up at the task level. The piece prior art doesn't have: tasks as a first-class node type in the provenance graph. Blame stops at commit; we go from commit to task to plan to ADR.

### §2.6 Knowledge management — bidirectional graph notes

Roam Research, Obsidian, Logseq. These tools made bidirectional links between markdown notes a first-class graph and built UIs around it (graph view, backlinks panel). Their key insight: most of the value isn't the graph view, it's the *automatic backlinks* — the inverse-edge rendering inside each note's surface.

Lineage's tool surface inherits this insight. The most-used closure isn't the graph traversal; it's the backlinks rendered when you load a task ("here are the tasks that cite this one"). What Roam-style tools don't have: typed edges, bound to executable artifacts, with derivation. They're hand-curated note graphs.

### §2.7 Software supply chain provenance

in-toto, SLSA (Supply-chain Levels for Software Artifacts), Sigstore Rekor. These shape attestation graphs over build artifacts: "this binary came from this commit, built by this runner, signed by this key, etc." The shape is similar — typed edges, audit-grade provenance, machine-derived rather than human-authored.

Lineage borrows the *shape* of provenance attestations: every edge carries (source, evidence, timestamp), and consumers can verify the source's claim against the evidence. What's different: lineage is a development-time reasoning surface, not a release-time integrity surface. Failure mode tolerance is much higher; we're trying to help an agent think, not prove a binary's pedigree.

### §2.8 IDE / programmer-assistance context aggregation

Sourcegraph Cody, Cursor, Cline, Continue, JetBrains AI Assistant, GitHub Copilot Workspace. All build context for an LLM by aggregating across files, symbols, recent commits, and (sometimes) issue references. Some surface "related issues" via heuristic matching. None expose the context-aggregation graph as a queryable artifact in its own right.

Lineage's authoring assist is structurally similar to context aggregation in these tools, but inverted: instead of the IDE deciding what context to pack and shipping it to the LLM as an opaque prompt, lineage exposes the graph the agent traverses *as the agent's tool*. The agent decides what to pull. This matches Orbit's philosophy of putting reasoning in the agent, not the wrapper.

---

## §3 What May Be Distinctive

Not novel: typed edges between tasks (Jira/Linear had this for a decade). Not novel: ADR supersession chains (Nygard, 2011). Not novel: traceability graphs (DOORS, since the 1980s). Not novel: link recovery from text (issue-tracker mining literature).

What may be genuinely new — or at least new in combination:

### 3.1 Durable agent memory keyed on code structure ("oral history for agents")

The framing in [1_overview.md §1](./1_overview.md): humans carry feature history in their heads; agents start each session blank. Every prior system in §2 was built for *humans* navigating *human-readable* surfaces — an issue-tracker sidebar, a graph view, a backlinks panel. Lineage is built for *agents* loading an *artifact* (a symbol, a file, a task) and getting that artifact's institutional context surfaced in one shot, in machine-readable form, with provenance the agent can audit.

The closest analogues — Sourcegraph code intelligence, Roam backlinks, in-toto attestations — each capture one piece (symbol cross-references, bidirectional link rendering, audit-grade provenance) but none stitch them into "the symbol's biography." That stitching is what the §5 closure surface and the §6 renderer in [2_design.md](./2_design.md) deliver. The bet: agent productivity is bottlenecked on context-recovery cost, and a substrate that pays that cost once and amortizes it across every future agent session is a step-function improvement, not a marginal one.

### 3.2 The bipartite task↔code bridge as the primary structure

Every prior system either binds links to code (provenance tools) or binds links to issues (issue trackers) — never both as one graph. Lineage's bipartite shape ([2_design.md §4](./2_design.md)) means "what tasks touched this symbol" and "what code did this lineage touch" are one query each, and they compose: "what code did the lineage of this symbol's tasks touch" is a one-shot SQL CTE. That compositionality is what makes §3.1 mechanically possible — without the bipartite bridge, biographies aren't keyable on symbols.

### 3.3 Derivation-first edges

Issue trackers have typed-link affordances; the data is empty in practice because humans don't link reliably. Lineage commits to deriving ≥80% of edges from existing signals (commit messages, KG attribution, ADR Status, runtime parent/child) and treats declared edges as the exception. This inverts the usual hand-authored-graph failure mode.

### 3.4 Deterministic, templated narrative rendering (§6 of [2_design.md](./2_design.md))

Most "knowledge surfacing" tools either dump a graph (expecting the human to read it) or summarize via LLM (introducing paraphrase risk). Lineage's renderer is templated and deterministic — it pulls strings from canonical fields and arranges them per a fixed grammar derived from the edge taxonomy. The cost is less polished prose; the gain is *auditability*. Every line of the rendered biography traces to a specific edge and a specific source row. This is the supply-chain-attestation shape applied to a development-time read surface, not just to provenance metadata.

### 3.5 Closure operations as the agent-facing surface

Most graph systems expose traversal primitives (vertex, edge, neighbor). Lineage's primary surface is *named closures* — `feature`, `biography`, `lineage`, `decision`, `reversal`, `risk` — each with a precise contract. Agents reason about the named closure, not about graph traversal mechanics. The raw `edges` enumeration is an escape hatch, not a recommended path.

### 3.6 Authoring write-back loop

Closures don't just *report*; they shape *future* task structure. The authoring-assist procedure ([2_design.md §7](./2_design.md)) recursively expands `context_files` through closure, and the resulting task carries a `lineage_seed` field — a reproducible record of what the agent considered. This converts "find all relevant files" from a search problem to a transitive-closure problem with audit trail.

### 3.7 Per-edge provenance + feedback loop

Every edge can be opened to its source row, line range, or commit. Wrong edges feed back to the *deriver*, not as one-off corrections. Over time, the derivation pipeline itself improves rather than the edge data improving. This is the supply-chain-provenance shape applied to a development-time graph.

### 3.8 Local-by-default with explicit cross-machine render

Lineage doesn't try to be a distributed graph. Each machine's graph is dense, fast, complete-for-its-window. Cross-engineer reach happens at *render time* via external_refs, not at graph time via replication. This honors task-sync's local-task-id doctrine and avoids reinventing distributed consistency for a development-time tool. ([Task-sync 2_design.md](../task-sync/2_design.md) is the substrate.)

### 3.9 Graph that improves the system that produced it

Phase 1's authoring assist already flips the graph from passive index to active shaping force — the lineage closure determines what `context_files` a new task carries forward. Phase 2 layers stale-task detection and ADR auto-supersession candidate triage as read-side helpers; Phase 3 goes further with decision archaeology and lineage-driven planning prompts. The graph grows into a feedback loop that improves the work that *produces* the graph. That recursive shape is, as far as we can tell, not present in any of the prior art surveyed.

(An earlier revision of this design sketched an auto-fire-on-PR "review assist" hook here. It was removed: on-demand `orbit.lineage.reversal` and `orbit.lineage.decision` closures cover the same reasoning when a reviewer or agent invokes them, without the banner-blindness risk of automated PR comments.)

The honest reading: each of these distinctive properties is small in isolation. Their composition — anchored on the §3.1 framing, with the phased roadmap that makes them load-bearing rather than aspirational — is where the bet lives. Phase 1's job is to prove the §6 biography surface carries weight; everything else is downstream of that proof.

---

## §4 References

### §4.1 Orbit-internal

- [docs/design/knowledge-graph/](../knowledge-graph/) — code-graph substrate; `task_ids` attribution restored in [4_decisions.md ADR-001](./4_decisions.md).
- [docs/design/task-sync/](../task-sync/) — local-task-id doctrine and external_refs spine.
- [docs/design/project-learnings/](../project-learnings/) — sibling system; potential third-node-type integration (§1.5).
- [docs/design/activity-job/](../activity-job/) — runtime parent/child edges via the run store.
- [docs/design/policy-sandbox/](../policy-sandbox/) — sandbox-aware traversal question (§1.6).
- [docs/design/auditability/](../auditability/) — provenance/audit conventions lineage borrows from.

### §4.2 External

- Nygard, M. (2011). "Documenting Architecture Decisions." Origin of the ADR pattern and supersession chain.
- in-toto / SLSA specifications. Provenance attestation graph shape.
- Roam Research / Obsidian / Logseq. Bidirectional-link knowledge graphs.
- DOORS, Polarion, Jama Connect documentation. Heavyweight requirements traceability.
- Wang, X. and Lo, D. "Detecting duplicate bug reports with software contextual information." Issue-link recovery research.
- GitHub Knowledge Graph / OSCAR. Issue-tracker mining at scale.
- Sourcegraph code-intelligence specifications. Symbol-level cross-repo provenance.

---

## Task References

- [T20260506-11] — Removed graph task attribution; supersession candidate ([4_decisions.md ADR-001](./4_decisions.md)).
- [T20260506-15] — Added `orbit.task.locks.reserve` `files` shape; canonical lineage exemplar.
- [T20260510-17] — Removed agent-facing lock instructions; the authoring failure that surfaced lineage.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
