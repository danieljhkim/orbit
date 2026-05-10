# Glossary: Task Lineage

Terms specific to the lineage system. Generic vocabulary (graph, edge, node, closure, recursive CTE, provenance) is excluded except where lineage gives a term a narrower meaning. Standard task store, knowledge graph, and ADR vocabulary lives in those features' own glossaries; this document covers only what's lineage-internal.

| Term | Meaning |
|------|---------|
| **Agent memory (durable)** | The framing in [1_overview.md §1](../1_overview.md): lineage exists to give discontinuous agent sessions the persistent symbol-grain context that human engineers carry in their heads. The biography surface is the read shape; the edge store is the substrate. |
| **Bipartite bridge** | The structural commitment that the task graph and the code knowledge graph are one graph with two node types, not two graphs that reference each other. See [2_design.md §4](../2_design.md), [4_decisions.md ADR-007](../4_decisions.md). |
| **Biography closure** | The closure operation (`orbit.lineage.biography`) that returns a rendered prose narrative for a code selector, alongside the structured `feature` payload. The headline read surface of the minimal Phase 1. See [2_design.md §5.2](../2_design.md), [§6](../2_design.md). |
| **Closure** | A named, opinionated graph-traversal projection. The minimal Phase 1 surface is `feature` and `biography`; additional closures are deferred. See [2_design.md §5](../2_design.md). |
| **Confidence** | Per-edge score in [0, 1] indicating derivation strength. Declared edges are 1.0; derived edges score by signal source. See [2_design.md §2.3](../2_design.md). |
| **Co-touches edge** | An edge between two tasks that share at least one file in `context_files` or commit attribution. The default file-grain edge. |
| **Co-touches-symbol edge** | A stronger version of `co-touches` that requires shared knowledge-graph node attribution rather than file-level overlap. |
| **Declared edge** | An edge whose `source` is `declared` — set explicitly by a human or agent, not inferred from a deriver. Confidence 1.0 by definition. |
| **Derivation cursor** | Per-deriver watermark stored in `derivation_cursors`, used to make the derivation pipeline incremental. See [2_design.md §2.4](../2_design.md). |
| **Derivation source** | One of `commit-grep`, `kg-attribution`, `task-text`, `declared` (minimal Phase 1). Stamped on every edge for provenance. Additional sources are deferred. |
| **Derived edge** | Any edge whose `source` is not `declared`. The dominant edge category by ADR-003 design. |
| **Deterministic renderer** | The Phase 1 biography renderer, which pulls strings from canonical fields and arranges them per a fixed grammar derived from the edge taxonomy — no LLM in the path. Auditable line-by-line. See [4_decisions.md ADR-009](../4_decisions.md), [2_design.md §6.2](../2_design.md). |
| **Edge type** | One of the typed relationships in [2_design.md §1.1](../2_design.md). The system's causal vocabulary; also the *grammar of the biography*. The minimal Phase 1 ships 7 task-task and 4 task-code types; others are named-and-deferred. |
| **Evidence** | The JSON payload stored on each edge that names the original source row, line range, commit SHA, or ADR section that justified the derivation. See [2_design.md §1.2](../2_design.md). |
| **Feature closure** | The closure operation that takes a code selector and returns all attributed tasks chronologically; the structured substrate the biography renderer consumes. See [2_design.md §5.1](../2_design.md). |
| **Open concerns** | Distinct from "honest limitations." The §9 list in [2_design.md](../2_design.md) of unresolved items the minimal Phase 1 has not yet decided on, ordered by ship-blocking priority — to be addressed one by one as follow-up tasks. |
| **Oral history (for agents)** | The metaphor in [1_overview.md §1.3](../1_overview.md): institutional context that humans pass informally and remember continuously is, for agents, replaced by symbol biographies attached to KG nodes and surfaced automatically on artifact load. |
| **Orphan biography** | A biography keyed on a KG `stable_id` that no longer survives the structural change it was meant to outlive (extract-into-multiple, inline-into-caller, cross-language port, wholesale rewrite). The renderer surfaces orphan cases via a footnote. See [2_design.md §7](../2_design.md). |
| **Symbol biography** | The rendered narrative attached to a KG node — chronologically-ordered, edge-typed, deterministic-templated. The headline read surface of the minimal Phase 1. See [2_design.md §6](../2_design.md), [4_decisions.md ADR-008](../4_decisions.md). |
| **Symbol identity stability** | The load-bearing assumption that KG `stable_id` survives the structural changes lineage is meant to outlive. Named explicitly so it can be challenged; failure modes documented in [2_design.md §7](../2_design.md), [4_decisions.md ADR-010](../4_decisions.md). |
| **Task-code edge** | An edge in `task_code_edges` between a task node and a code-graph node. Distinct from task-task edges in `task_edges`. |
| **Verb upgrade** | The `task-text` deriver's mechanism for promoting a default `references` edge to a typed edge (`supersedes`, `extends`, `reverts`, `motivates`) when an adjacent verb in the source text matches an upgrade rule. The renderer's primary source of narrative verbs; ship-blocking eval set in [2_design.md §9.1](../2_design.md). See [2_design.md §2.2](../2_design.md). |
