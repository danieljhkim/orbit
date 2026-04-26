# Knowledge Graph — Vision

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-04-26 (read-only public graph surface, [T20260426-0453])

This document captures where the knowledge graph is headed: open questions, where it fits relative to prior art, and what may turn out to be distinctive about Orbit's shape. See [1_overview.md](./1_overview.md) for the system's purpose and [2_design.md](./2_design.md) for what exists today.

Treat everything below as a hypothesis, not a commitment. Items here do not carry task IDs because they are not yet scheduled work; if an item lands, the task reference will appear in [2_design.md](./2_design.md) when that document is updated.

---

## 1. Open Questions

### 1.1 Cross-language reference resolution

Is it worth a per-language type checker pass? Or is the right shape a pluggable "reference provider" trait, with LSP as one optional backend? Current caller/implementor resolution ([T20260412-0645-3]) is signature-matching, not type-resolved — precise enough for navigation, a superset for safety-critical refactors (see §6.2 in [2_design.md](./2_design.md)).

### 1.2 Structural diff surface

We attribute task IDs to nodes ([T20260421-0528]). Should we also attribute per-node *change kind* — added / modified / deleted per commit — as a first-class field, so `orbit.graph.show` can render a symbol's recent evolution without re-walking git?

### 1.3 Working-graph persistence on crash

The working graph is currently internal/deferred and in-memory ([T20260411-0424], [T20260426-0453]). If public graph mutation returns, a long activity that crashes mid-edit would lose its staging. Do we persist the working graph to disk under `.orbit/knowledge/working/<activity_id>/` and replay on restart? If so, how does the on-disk working copy interact with branch switches during recovery?

### 1.4 Semantic embeddings as an additional index

The graph is symbolic and structural. Natural-language queries ("where do we handle auth failures") today degrade to substring search. Is there a clean way to layer embedding vectors onto leaves without coupling to a specific provider, and without duplicating the content-addressed store? An earlier attempt at semantic indexing ([T20260408-0445], archived) staged the shape of this but was parked when structural queries proved sufficient for the current agent workloads.

### 1.5 Rename tracking across history

§6.3 in [2_design.md](./2_design.md) — accept the current best-effort, or invest in `--follow`-equivalent hunk re-mapping? The cost compounds with every rename hop, which is why the walker in [T20260421-0528] opted out. Two archived predecessors ([T20260421-0342], [T20260421-0343]) explored persistent task→symbol edges with rename survival and were parked in favor of the identity-match-only approach that shipped; revisit them if rename blindness proves material.

### 1.6 Cross-workspace graph sharing

If two Orbit workspaces point at different branches of the same repo, can they share the object/blob store? The content-addressed layout makes this theoretically free; the integration points (refs, manifests, locks, ref migration from [T20260421-0358]) need design.

### 1.7 Incremental leaf extraction

Today a modified file re-extracts every leaf. For large files (think 3000-line modules) this is wasteful. Is there a shape where unchanged hunks preserve their prior leaves without re-running tree-sitter? Related to the speedup work in [T20260417-0639] but deeper: that task optimized persistence, not extraction.

### 1.8 Locking vocabulary

`is_locked` vs `lineage_locked` is two flags ([T20260411-0424], hardened in [T20260417-0301-2]); is that enough? Does a "review-locked" state (don't rename, but do allow body edits within strict invariants) belong?

### 1.9 Pack rendering budget management

`pack_json` takes a token budget; the packing heuristic is currently hand-tuned. Is there a win from teaching the packer about which nodes the agent has already seen in-session, so it skips re-including them?

### 1.10 Garbage collection

§6.7 in [2_design.md](./2_design.md) — what's the right reachability definition, and who triggers GC (background, explicit `orbit gc`, on-demand during build)?

### 1.11 Shipped-vs-WIP attribution

§6.8 in [2_design.md](./2_design.md). The flat union of `task_ids` on a node loses the signal that matters most: *which task that touched this symbol actually shipped*. Cleanest shape is probably to filter at query time from task status, but that requires the graph to depend on task state, which is a layering decision.

---

## 2. Prior Work

The knowledge graph is a synthesis of well-known patterns, not a novel primitive. The components below exist in the 2023–2026 literature and in production tooling; Orbit's contribution is the specific combination tuned for agent prompt assembly and the git-native ref model.

### 2.1 Code graphs for static analysis

- **GitHub CodeQL / Semmle** — queryable code graphs with a relational model. Strong semantic fidelity; heavy per-language extractor investment; queries are their own DSL.
- **Sourcegraph SCIP / LSIF** — language-agnostic indexes produced by per-language tools, designed for cross-repo navigation. SCIP's design for diff-friendly incremental indexes informed Orbit's attribution pass ([T20260421-0528]).
- **Glean (Meta)** — production graph store for code facts over many languages. Shares the "content-addressed facts + query layer" shape.

Orbit's graph is structurally simpler than any of these — directory/file/leaf with signatures, not full type-resolved references. The trade is extractor maintenance cost vs. query precision.

### 2.2 Tree-sitter extraction

- **tree-sitter** (Brunsfeld et al.) — the de facto parser framework for editor and indexer tooling. Orbit's extractors are thin adapters over language-specific grammars.
- **ctags / universal-ctags** — the pre-tree-sitter analog. Still widely used; its tag kinds directly inspired Orbit's `LeafKind` vocabulary (function, method, class, struct, interface, field, module).

Nothing in the extractor layer is novel; we use it as off-the-shelf infrastructure.

### 2.3 Content-addressed storage for code state

- **git** — the direct model for objects/refs/index. Orbit deliberately uses the same split ([T20260421-0358]) to keep operator intuition correct.
- **IPFS / dat** — content-addressed distribution. Not directly influential but confirms the pattern's generality.

### 2.4 Agent-oriented code indexes

- **Cursor / Continue / Cline repo maps** — hand-rolled repo summarization for prompt injection. Generally one-shot; not branch-scoped, not incremental, not history-attributed.
- **Aider repo map** — ranked file/symbol summary generated per request. Cheaper than a full graph, less precise; no persistence across sessions.
- **Sweep / CodePlan / Agentless** — research agents that build ad-hoc code graphs before planning. Each rebuilds from scratch; none persist a ref model.
- **Symbex / Chapter** — local semantic search over code. Symbol-level but embedding-first rather than structure-first.
- **Graphify** ([safishamsi/graphify](https://github.com/safishamsi/graphify)) — agent-installable skill (Claude Code, Codex, Cursor, and others) that ingests a folder of code, docs, PDFs, images, and video/audio into a single queryable knowledge graph. Tree-sitter AST extraction across ~25 languages, Whisper for transcripts, Claude subagents for concept and relationship extraction, Leiden community detection for clustering, and explicit `EXTRACTED` / `INFERRED` / `AMBIGUOUS` edge tagging. Orbit's graph did not draw from Graphify's design — the two projects converged on tree-sitter independently — but the overlap is worth naming: Graphify is scoped at *"make any folder queryable for any assistant,"* Orbit is scoped at *"make one workspace queryable for one orchestrator across branches with a ref model."* Both choose graph topology over embeddings for similarity signal; neither needs a vector DB.

Orbit's direction differs primarily in **persistence, branch-awareness, and scope**: the graph is a durable workspace artifact keyed to a git ref, not a per-session or per-folder computation, and it is the same artifact every tool and every activity reads from. Graphify's multimodal reach (videos, whiteboards, PDFs) is deliberately out of Orbit's scope today — Orbit indexes code and code-adjacent structure only.

### 2.5 LSP as a foil, not a target

The Language Server Protocol would give us reference resolution for free. Orbit does not use it because:

1. LSPs are stateful processes; the graph is a file-on-disk artifact. Querying an LSP from an agent tool adds lifecycle complexity (spawn, warm up, dispose) that a file read does not.
2. LSP responses are tuned for interactive UX; prompt assembly wants bulk, structured, token-budgeted output.
3. Multi-language coverage requires N+1 servers, each with its own startup cost.

A future reference-provider abstraction (§1.1) could make LSP an optional backend without forcing it to be the default.

---

## 3. What May Be Distinctive

Softened claims after survey:

- **Branch-scoped refs over a shared content-addressed store** ([T20260421-0358]). This specific combination — multi-worktree safe, concurrent-build safe, read-on-missing-ref-falls-back-to-default — is not something we have seen packaged in an agent-facing code graph. Close analogs (SCIP, Glean) are server-backed; Orbit does it file-on-disk.
- **Task-ID attribution as a first-class node field** ([T20260421-0528]). Most code graphs index *authors* and *timestamps*. Keying to a task identifier is Orbit-specific and load-bearing for the lifecycle scoreboard.
- **Working-graph overlay as the mutation surface** ([T20260411-0424]). Separating "the published graph that all reads see" from "the in-flight edits of a single activity" is a clean shape; whether it survives contact with long, crash-prone activities is an open question (§1.3).

None of these rise to a research contribution. Treat the knowledge graph as productization of known primitives, with opinionated defaults for an agent-execution context.

---

## 4. References

### Orbit-internal
- [1_overview.md](./1_overview.md) — motivation and core concepts
- [2_design.md](./2_design.md) — current implementation
- [specs/refs.md](./specs/refs.md) — ref resolution, migration, concurrency
- [../activity-job/2_design.md](../activity-job/2_design.md) — the activity/job model that coordinates task execution and preflight guards
- `crates/orbit-knowledge/` — implementation

### External
- tree-sitter — https://tree-sitter.github.io/tree-sitter/
- SCIP — https://github.com/sourcegraph/scip
- LSIF — https://microsoft.github.io/language-server-protocol/specifications/lsif/0.4.0/specification/
- CodeQL — https://codeql.github.com/
- Glean — https://glean.software/
- universal-ctags — https://github.com/universal-ctags/ctags
- Aider repo map — https://aider.chat/docs/repomap.html
- Graphify — https://github.com/safishamsi/graphify

---

## Task References

Tasks cited in this document (all as forward pointers or historical context; none are proposed work on this doc):

- **[T20260408-0445]** (archived) — Earlier semantic-indexing attempt; context for §1.4.
- **[T20260411-0424]** — Working-graph mutation internals and lock store; foundation for §1.3 and §1.8.
- **[T20260412-0645-3]** — Architectural graph navigation (`callers`, `implementors`, `deps`); foundation for §1.1.
- **[T20260417-0301-2]** — Lock/write/read hardening.
- **[T20260417-0639]** — Persistence-path speedup; related to §1.7.
- **[T20260421-0342]** (archived) — Symbol-level git-log-based task lookup; superseded by attribution-on-node.
- **[T20260421-0343]** (archived) — Indexed task→symbol edges with rename survival; superseded by identity-match attribution.
- **[T20260421-0358]** — Branch-scoped refs; foundation for §3's distinctiveness claim and §1.6.
- **[T20260421-0528]** — History-walker + `task_ids` attribution; foundation for §1.2, §1.5, and §3.
- **[T20260426-0453]** — Current public graph surface is read-only; write coordination uses task lock reservations.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
