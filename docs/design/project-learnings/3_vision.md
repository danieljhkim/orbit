---
summary: "Project Learnings — Vision"
type: design
title: "Project Learnings — Vision"
owner: claude
last_updated: 2026-08-10
last_validated: 2026-08-13
status: Draft
feature: project-learnings
doc_role: vision
tags: ["project-learnings"]
---

# Project Learnings — Vision

This document captures the questions phase 1 deliberately defers, the prior work the design draws on or rejects, what is specific to Orbit's situation, and external references for further reading. The questions in §1 are the most likely sources of post-phase-1 design pressure.

---

## 1. Open Questions

### 1.1 Symbol-aware scope (unplanned)

Phase 1 scopes learnings by path globs and tags ([2_design.md §3](./2_design.md)). This breaks under renames: a learning scoped to a benchmark source file becomes invisible the moment someone moves the file, even though the knowledge is still about the same logic.

The schema preserves a `scope.symbols:
["orbit-engine::perf_runner::run_benchmark"]` field, but Orbit has no live
symbol resolver. Reasons the field remains inactive:

- A resolver would add a new indexing dependency and lifecycle surface.
- Symbol-aware scope is more useful once semantic-similarity ranking exists ([§1.2](#12-semantic-similarity-ranking-deferred-to-phase-2)), because the two together give "find learnings about this symbol or anything semantically near it."
- The phase-1 schema reserves `scope.symbols`, so a future design can remain
  additive rather than requiring a data migration.

**Cost of deferring:** every refactor that moves files requires manual `orbit learning prune` or `update` calls. At low learning volume that's tolerable; at higher volume it becomes drag.

### 1.2 Semantic-similarity ranking and remaining gaps

The indexed lexical path ranks matched learnings by priority and recency, while opt-in hybrid search can add semantic similarity after learnings are indexed ([2_design.md §8.3](./2_design.md)). The remaining gaps are predictable:

- An old, important learning loses to a recent, marginal one.
- A query that's semantically related to a learning gets no semantic result unless the learning vector index has been built.
- Lexical and semantic candidates can still differ in quality for the *current* query, especially when the local companion or vectors are unavailable.

[docs/design/orbit-search/](../orbit-search/) provides per-field embeddings, cosine retrieval, and hybrid fusion. The current learning search path uses that infrastructure:

- Each learning's `summary` and `body` are embedded under the same `embeddings` table orbit-search uses (`source_kind = "learning"`).
- Search-time ranking combines lexical candidates with cosine matches against the query, then applies the requested path and tag filters.
- Manual `priority` orders the lexical store results; hybrid ranking blends normalized lexical and semantic scores.

The remaining follow-up is symbol-aware scope. `scope.semantic_seed` remains a forward-compatible field, while current learning embeddings use the summary, body sections, and tags.

**Cost of the remaining gap:** semantic retrieval requires an explicit local index and companion, while symbol-aware scope would add resolver and lifecycle complexity. Lexical search remains the fallback when semantic retrieval is unavailable.

### 1.3 Authoring incentives and lag

The whole system depends on someone writing the learnings. Phase 1 ships:

- The bundled `orbit-knowledge` skill for learning authoring and curation, alongside `orbit-search` for retrieval.
- A CLI/MCP surface for direct invocation.
- A hand-curation expectation: humans write learnings during PR review or after incidents.

None of these guarantee learnings get authored. Three candidate accelerators, all out of scope phase 1:

- **Auto-suggestion at task close.** When a task is approved (`orbit.task.approve`, or `orbit task update --status done` out of review), surface "did the agent learn anything from this task that should become a learning?" Adds friction; may help.
- **Mining from review threads.** Crawl resolved review threads for sentences matching patterns like "remember to" / "always" / "don't" / "we got burned" and suggest them as draft learnings. Cheap to implement, high-noise without a relevance filter.
- **Mining from MEMORY.md.** Agent-private memory often contains lessons that should be project-wide. A migration tool ("promote this MEMORY.md entry to a project learning") would convert quietly-accumulating private knowledge into shared artifact.

The third is the most promising — it converts existing material rather than generating new — and has the cleanest UX ("review and elevate"). Likely picked up alongside or after phase 2.

### 1.4 Cross-workspace learnings

Phase 1 is workspace-scoped per [CLAUDE.md](../../../CLAUDE.md)'s Scoping Rules table. A learning written in repo A doesn't surface for repo B. This is correct for most learnings ("the perf_runner module needs equivalence checks" only applies to this repo) but wrong for some ("never declare perf wins on latency alone" generalizes).

Three options:

- **Status quo: workspace-only, accept the duplication.** Each repo accumulates its own copy of cross-cutting learnings. High redundancy; zero coordination cost.
- **Global learnings under `~/.orbit/learnings/`.** Mirrors the global skill scoping. Risk: global learnings drift from any specific repo's reality.
- **Tag-driven promotion.** Mark a learning `cross_workspace: true`; a separate `~/.orbit/learnings/` is populated by promoted records. Operator opts in.

The third is probably right; phase 1 ships option 1.

Note that the per-machine coordination model raises the price of options 2 and 3. Learnings are now keyed `(workspace_id, artifact_key)` with no global tier anywhere in the system ([../host-registry/4_decisions.md](../host-registry/4_decisions.md) ADR-0357), so a `~/.orbit/learnings/` store would be reintroducing the cross-workspace namespace that was just removed — and on a multi-machine constellation it would need an owner and a merge story of its own. Either option now needs its own decision rather than arriving as later work.

### 1.5 Pull discovery quality

[ORB-10346] retired the Claude Code `PreToolUse` hook layer after the relevancy audit showed it added broad tool-call overhead without a useful direct signal, but left the other two automatic-delivery layers active: engine pre-prompt injection and the MCP sidecar decorator (source locations and status: [4_decisions.md ADR-0108 amendment](./4_decisions.md)). The discovery model below layers on top of that live push delivery, not in its absence.

- **Search and show only.** Lowest runtime overhead and portable across every agent, but relies on the query being meaningful.
- **Search and show plus reference comments.** A concise artifact ID and rationale at a code or workflow boundary give agents a concrete locator before they retrieve the authoritative body.
- **Restore the Claude Code hook layer.** Reopens the audit's low-relevancy, vendor-locked hot path; it needs evidence beyond the frozen historical counters.

Phase 1 uses search/show plus reference comments, on top of the two still-active push layers. Future ranking work should improve the pull results rather than reintroduce the Claude Code hook.

### 1.6 Privacy of learning content under shared repos

Learnings are checked in. In a public open-source repo, every learning is public. Most learnings are fine to share ("never declare perf wins on latency alone"); some may not be ("our auth subsystem has a known race in X — rewrite incoming"). Phase 1 has no `private: true` flag.

Two paths if this becomes load-bearing:

- A `private` flag plus a separate `.orbit/learnings/private/` directory that's `.gitignore`d. Operator-driven.
- A redaction layer that filters or sanitizes retrieval results in untrusted contexts. Heavier; probably overkill until a real use case appears.

Phase 1 ships nothing here and flags the consideration; if the project becomes a multi-tenant or open-source codebase, this is the section to revisit first.

### 1.7 Interaction with the friction-bounty scoreboard

Friction reports ([CLAUDE.md](../../../CLAUDE.md) §"Friction Reports") and learnings overlap conceptually: both capture "something an agent hit and wants future agents to know about." The scoreboard rewards friction reports. Should learnings authored by agents also count toward a scoreboard?

Arguments for: yes, authoring is the bottleneck ([§1.3](#13-authoring-incentives-and-lag)); rewarding it directly is the fastest accelerator.
Arguments against: scoreboard incentives produce volume, not quality, and learnings have a higher quality bar than friction reports (which are inherently first-person and time-stamped).

Out of scope phase 1; flagged because the Friction Reports section is the closest existing model for agent-authored project artifacts.

### 1.8 Format evolution and `schemaVersion`

The YAML schema declares `schemaVersion: 1`. Anticipated changes:

- v2: add `scope.symbols` ([§1.1](#11-symbol-aware-scope-deferred-to-phase-2)).
- v2 or v3: add `scope.semantic_seed` ([§1.2](#12-semantic-similarity-ranking-deferred-to-phase-2)).
- Possibly: add `confidence` (low/medium/high) for ranking.
- Possibly: add `audience` (agent/human/both) for filtering retrieval results.

Migrations follow the same pattern as task `schemaVersion: 2` — additive when possible, with a one-shot migrator otherwise. The cost line: every schema bump is operationally non-trivial because YAML records are checked in and PRs from before the bump may need rebasing.

---

## 2. Prior Work

### 2.1 Internal precedents

- **Agent `MEMORY.md`** — the per-agent feedback/preference store this design is modeled after. Project-learnings makes the same kind of knowledge project-shared and retrievable through a durable registry.
- **Friction Reports** ([CLAUDE.md](../../../CLAUDE.md)) — agent self-reports of tooling problems. Same authoring shape, different content focus (process pain vs. project knowledge). The friction-bounty scoreboard is a precedent for incentivizing agent-authored artifacts.
- **ADR logs** ([docs/design/CONVENTIONS.md](../CONVENTIONS.md) §4) — the closest existing artifact for "non-obvious decisions a future reader needs." Different shape: ADRs are feature-scoped, decision-shaped, and human-curated; learnings are cross-cutting, rule-shaped, and agent-or-human authored.
- **gstack `/learn` skill** — a pull-oriented project-learning store. It is a useful precedent for explicit retrieval; Orbit adds structured records, searchable metadata, and point-of-use reference comments.

### 2.2 External precedents

- **Runbooks and operational playbooks.** The closest industry pattern: durable, explicitly retrieved guidance organized around an operator's question.
- **Linter rules and ESLint custom plugins.** An appropriate push mechanism for mechanical rules that can be checked deterministically. Project learnings retain natural-language judgment and do not impersonate a linter.
- **CodeQL queries / Semgrep rules.** Programmatic "remember to" rules. Strong for what they cover (mechanical patterns); they don't capture the wider class of judgment-shaped knowledge ("never declare a perf win on latency alone" is hard to express as a regex).
- **Notion/Obsidian/Confluence project wikis.** Same content domain, but their vocabulary mismatch makes the right page hard to find. Orbit uses structured scope metadata and nearby reference comments to improve the locator.
- **Continue.dev / Cursor "rules" files.** Vendor-specific configuration files that prepend instructions to prompts. They remain a useful contrast: coarse and vendor-locked automatic context rather than explicit, portable retrieval.

### 2.3 What was rejected

- **Flat markdown directory** (`docs/learnings/*.md`). Easy to author, impossible to query at agent runtime. Rejected as the storage substrate; see [4_decisions.md ADR-002](./4_decisions.md).
- **The Claude Code `PreToolUse` hook layer of automatic delivery.** Retired after the 2026-07-18 relevancy audit; the historical injection counters are retained only as calibration data. Engine pre-prompt injection and the MCP sidecar decorator, the other two automatic-delivery layers, remain active. See [2_design.md §4.3](./2_design.md).
- **CLAUDE.md fragments**. Loaded on every session regardless of relevance. Pollutes context for unrelated work. Rejected; learnings need scope filtering.
- **Workspace-private storage** (under `.orbit/state/` only, not checked in). Loses cross-collaborator value; same defect as agent `MEMORY.md` for this content type. See [4_decisions.md ADR-003](./4_decisions.md).

---

## 3. What May Be Distinctive

Three properties separate this design from the prior art it draws on.

### 3.1 Pull delivery with point-of-use locators

Wikis are often hard to retrieve because the reader must guess the vocabulary. Project-learnings keeps the authoritative guidance searchable and supplements it with a compact reference comment where a recurring boundary needs explanation. It stays vendor-neutral, avoids per-tool-call overhead, and does not duplicate the durable body into source.

### 3.2 Native to the dev-loop infrastructure

Most "team knowledge base" tools live outside the dev loop — a separate web app, a wiki, a chat channel. Project-learnings lives in `.orbit/learnings/` next to `.orbit/tasks/`, with the same lifecycle, git semantics, and MCP/CLI retrieval surface. A reference comment at a code or workflow boundary keeps the registry connected to the work without serving content automatically.

### 3.3 Lifecycle bound to code via explicit scopes

A learning becomes a staleness candidate when its path scope or cited
task/commit evidence no longer resolves. Checks are opportunistic and pruning
remains explicit, keeping lifecycle behavior deterministic without a background
indexer.

---

## 4. References

### 4.1 Orbit-internal

- [docs/design/CONVENTIONS.md](../CONVENTIONS.md) — folder layout, frontmatter, ADR template.
- [docs/design/orbit-search/](../orbit-search/) — local semantic-index and hybrid-ranking implementation used by learning search.
- [docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/) —
  historical context for the retired symbol-aware proposal.
- [docs/design/_archive/task-sync/](../_archive/task-sync/1_overview.md) — relevant for whether learnings should sync across machines (decision: yes, via the same checked-in path tasks use). Archived; superseded by [remote-access](../remote-access/1_overview.md).
- [CLAUDE.md](../../../CLAUDE.md) — friction-reports section is the closest existing precedent for agent-authored project artifacts.
- `orbit-knowledge` skill — the bundled authoring shape for durable project learnings.

### 4.2 External

- **Continue.dev `rules` files** — `https://docs.continue.dev/customization/rules`. Vendor-specific contrast to Orbit's explicit retrieval model.
- **Cursor `.cursorrules`** — same shape, different vendor. Cited as evidence the form is in demand and as an example of why a cross-agent design is needed.
- **Reciprocal Rank Fusion (Cormack, Clarke, Büttcher 2009)** — same fusion algorithm orbit-search uses; relevant once phase 2 fuses path-glob matches with cosine matches.
- **"The Documentation System" / Diátaxis framework** — `https://diataxis.fr/`. Useful taxonomy for what *isn't* a learning (tutorials, reference, how-to, explanation) and therefore what belongs elsewhere.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [ORB-10346] — Retired the Claude Code `PreToolUse` hook layer; engine pre-prompt injection and the MCP sidecar decorator remain active alongside pull discovery and reference comments.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
