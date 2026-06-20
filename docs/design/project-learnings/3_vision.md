---
summary: "Project Learnings — Vision"
type: design
title: "Project Learnings — Vision"
owner: claude
last_updated: 2026-05-12
status: Draft
feature: project-learnings
doc_role: vision
tags: ["project-learnings"]
---

# Project Learnings — Vision

This document captures the questions phase 1 deliberately defers, the prior work the design draws on or rejects, what is specific to Orbit's situation, and external references for further reading. The questions in §1 are the most likely sources of post-phase-1 design pressure.

---

## 1. Open Questions

### 1.1 Symbol-aware scope (deferred to phase 2)

Phase 1 scopes learnings by path globs and tags ([2_design.md §3](./2_design.md)). This breaks under renames: a learning scoped to `crates/orbit-knowledge/src/graph_bench.rs` becomes invisible the moment someone moves the file, even though the knowledge is still about the same logic.

The knowledge graph already tracks symbol identity across moves. A `scope.symbols: ["orbit-engine::perf_runner::run_benchmark"]` field would survive renames cleanly because the graph resolves symbols regardless of file location. Reasons phase 1 doesn't ship this:

- Coupling the learning store to the knowledge graph adds a hot-path dependency on graph rebuilds. Phase 1 keeps the dependency one-way (graph triggers staleness checks) rather than bidirectional.
- Symbol-aware scope is more useful once semantic-similarity ranking exists ([§1.2](#12-semantic-similarity-ranking-deferred-to-phase-2)), because the two together give "find learnings about this symbol or anything semantically near it."
- The phase-1 schema reserves `scope.symbols` so adding it later is additive, not a migration.

**Cost of deferring:** every refactor that moves files requires manual `orbit learning prune` or `update` calls. At low learning volume that's tolerable; at higher volume it becomes drag.

### 1.2 Semantic-similarity ranking (deferred to phase 2)

Phase 1 ranks matched learnings by `updated_at` desc, with optional manual `priority` tagging ([2_design.md §8.3](./2_design.md)). This is wrong in predictable ways:

- An old, important learning loses to a recent, marginal one.
- A query that's semantically related to a learning but doesn't match its path globs gets nothing.
- Multiple matched learnings with the same path scope all rank by date, with no signal about which is most relevant to the *current* edit.

[docs/design/orbit-search/](../orbit-search/) builds the infrastructure that resolves all three: per-field embeddings, brute-force cosine, RRF fusion. Phase 2 of project-learnings layers on top of that:

- Each learning's `summary` and `body` are embedded under the same `embeddings` table orbit-search uses (`source_kind = "learning"`).
- Injection-time ranking unions path-glob matches with cosine matches against the current edit's surrounding context, fused via RRF.
- Manual `priority` becomes a soft signal in fusion, not a hard tier.

The phase-2 design lands as its own task once orbit-search reaches Accepted. The schema reservation in phase 1 is a `scope.semantic_seed` field — short text describing what the learning is "about" — that becomes the embedding source for phase 2.

**Cost of deferring:** the phase-1 ranking is a placeholder. Users will likely hit the recency-blindness failure mode before phase 2 ships, and the documented mitigation is "use `--limit 5` and let humans curate."

### 1.3 Authoring incentives and lag

The whole system depends on someone writing the learnings. Phase 1 ships:

- An `orbit-learnings` skill that walks an agent through authoring at task close.
- A CLI/MCP surface for direct invocation.
- A hand-curation expectation: humans write learnings during PR review or after incidents.

None of these guarantee learnings get authored. Three candidate accelerators, all out of scope phase 1:

- **Auto-suggestion at task close.** When `orbit task approve` runs, surface "did the agent learn anything from this task that should become a learning?" Adds friction; may help.
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

### 1.5 Cross-agent hook universality

[2_design.md §4.3](./2_design.md) ships layer 3 as Claude-Code-only because that's the only agent vendor with a documented `PreToolUse` hook surface today. Codex, Gemini, and others may gain similar facilities; some won't. Three responses:

- **Wait for vendor parity.** Accept uneven coverage until each vendor adds hooks. Slowest; least Orbit work.
- **Layer 4: an Orbit-side proxy.** Run the agent's tool calls through an Orbit interceptor that simulates `PreToolUse` for any agent. Adds a new component in the hot path; large surface area.
- **Push fine-grained injection up to layer 1+2.** Make engine-pre-prompt and MCP-sidecar smart enough that the per-edit precision of layer 3 is rarely needed.

The third is the most palatable; it depends on how good ranking gets in phase 2. If phase-2 ranking is strong, the gap layer 3 fills shrinks; if it's weak, layer 4 becomes more attractive.

### 1.6 Privacy of learning content under shared repos

Learnings are checked in. In a public open-source repo, every learning is public. Most learnings are fine to share ("never declare perf wins on latency alone"); some may not be ("our auth subsystem has a known race in X — rewrite incoming"). Phase 1 has no `private: true` flag.

Two paths if this becomes load-bearing:

- A `private` flag plus a separate `.orbit/learnings/private/` directory that's `.gitignore`d. Operator-driven.
- A redaction layer that injects sanitized summaries into agents in untrusted contexts. Heavier; probably overkill until a real use case appears.

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
- Possibly: add `audience` (agent/human/both) for filtering injection.

Migrations follow the same pattern as task `schemaVersion: 2` — additive when possible, with a one-shot migrator otherwise. The cost line: every schema bump is operationally non-trivial because YAML records are checked in and PRs from before the bump may need rebasing.

---

## 2. Prior Work

### 2.1 Internal precedents

- **Agent `MEMORY.md`** — the per-agent feedback/preference store this design is modeled after. Project-learnings extends the same idea (push-based discovery via auto-loading) from agent-private to project-shared, and from session-context-load to per-action injection.
- **Friction Reports** ([CLAUDE.md](../../../CLAUDE.md)) — agent self-reports of tooling problems. Same authoring shape, different content focus (process pain vs. project knowledge). The friction-bounty scoreboard is a precedent for incentivizing agent-authored artifacts.
- **ADR logs** ([docs/design/CONVENTIONS.md](../CONVENTIONS.md) §4) — the closest existing artifact for "non-obvious decisions a future reader needs." Different shape: ADRs are feature-scoped, decision-shaped, and human-curated; learnings are cross-cutting, rule-shaped, and agent-or-human authored.
- **gstack `/learn` skill** — pull-only project-learnings store. Cited as the failure mode this design improves on: pull-only requires agent discipline to query, which is the exact failure mode push-injection prevents.

### 2.2 External precedents

- **Runbooks and operational playbooks.** The closest industry pattern. Runbooks are typically pull-only and topic-organized; the push-injection layer here is what makes the form useful at the moment of action rather than at the moment of question.
- **Linter rules and ESLint custom plugins.** Push-based delivery (the linter fires on save) for code-shaped knowledge. Project-learnings extends the form to natural-language knowledge that doesn't compile down to a lint rule.
- **CodeQL queries / Semgrep rules.** Programmatic "remember to" rules. Strong for what they cover (mechanical patterns); they don't capture the wider class of judgment-shaped knowledge ("never declare a perf win on latency alone" is hard to express as a regex).
- **Notion/Obsidian/Confluence project wikis.** Same content domain, pull-only delivery. The vocabulary-mismatch problem dominates ("I searched for X and the right page didn't come up"). Project-learnings sidesteps it by pushing rather than waiting for the agent to query.
- **Continue.dev / Cursor "rules" files.** Vendor-specific configuration files that get prepended to every agent prompt. Closest in spirit to layer-1 push-injection. Differences: rules-files are coarse (one file, all agents, all tasks), un-scoped (no path-glob filter), and vendor-locked (each editor has its own format).

### 2.3 What was rejected

- **Flat markdown directory** (`docs/learnings/*.md`). Easy to author, impossible to query at agent runtime. Rejected as the storage substrate; see [4_decisions.md ADR-002](./4_decisions.md).
- **Pull-only via search tool**. Discoverability falls to agent discipline; the failure mode being prevented. See [4_decisions.md ADR-001](./4_decisions.md).
- **CLAUDE.md fragments**. Loaded on every session regardless of relevance. Pollutes context for unrelated work. Rejected; learnings need scope filtering.
- **Workspace-private storage** (under `.orbit/state/` only, not checked in). Loses cross-collaborator value; same defect as agent `MEMORY.md` for this content type. See [4_decisions.md ADR-003](./4_decisions.md).

---

## 3. What May Be Distinctive

Three properties separate this design from the prior art it draws on.

### 3.1 Push-based delivery for natural-language knowledge

Linter rules push (they fire on save). Wikis pull (you have to look). Project-learnings is push-based for natural-language, judgment-shaped knowledge — a combination the prior art doesn't cover. The closest analog is vendor-specific "rules files" (Cursor, Continue), but those are unscoped and vendor-locked; the design here is scope-filtered and cross-agent.

### 3.2 Native to the dev-loop infrastructure

Most "team knowledge base" tools live outside the dev loop — a separate web app, a wiki, a chat channel. Project-learnings lives in `.orbit/learnings/` next to `.orbit/tasks/`, with the same lifecycle, the same git semantics, and the same MCP surface. The friction of authoring is whatever the agent's friction-of-running-an-Orbit-tool-call is, which is roughly zero. The friction of consumption is also roughly zero, because injection is automatic.

### 3.3 Lifecycle bound to code via the knowledge graph

A learning that references a function the graph no longer recognizes is flagged stale automatically. This is a small thing, but it directly attacks the most common failure mode of long-lived knowledge bases: stale content that's still being served. The phase-1 implementation is conservative (opportunistic checks, manual prune); phase 2 ties it more tightly into the graph rebuild path.

---

## 4. References

### 4.1 Orbit-internal

- [docs/design/CONVENTIONS.md](../CONVENTIONS.md) — folder layout, frontmatter, ADR template.
- [docs/design/orbit-search/](../orbit-search/) — phase 2 dependency for semantic-similarity ranking.
- [docs/design/_archive/knowledge-graph/](../_archive/knowledge-graph/) — phase 2 dependency for symbol-aware scope and staleness detection.
- [docs/design/task-sync/](../task-sync/) — relevant for whether learnings should sync across machines (decision: yes, via the same checked-in path tasks use).
- [CLAUDE.md](../../../CLAUDE.md) — friction-reports section is the closest existing precedent for agent-authored project artifacts.
- `orbit-create-task` skill (`~/.claude/skills/orbit-create-task/`) — the authoring shape `orbit-learnings` will mirror.

### 4.2 External

- **Continue.dev `rules` files** — `https://docs.continue.dev/customization/rules`. Vendor-specific approximation of layer-1 push injection.
- **Cursor `.cursorrules`** — same shape, different vendor. Cited as evidence the form is in demand and as an example of why a cross-agent design is needed.
- **Anthropic Claude Code hooks** — `https://docs.claude.com/en/docs/claude-code/hooks`. The mechanism layer 3 uses.
- **Reciprocal Rank Fusion (Cormack, Clarke, Büttcher 2009)** — same fusion algorithm orbit-search uses; relevant once phase 2 fuses path-glob matches with cosine matches.
- **"The Documentation System" / Diátaxis framework** — `https://diataxis.fr/`. Useful taxonomy for what *isn't* a learning (tutorials, reference, how-to, explanation) and therefore what belongs elsewhere.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
