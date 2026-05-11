# Project Learnings — Decisions

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-11

ADR-style log of non-obvious project-learnings decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are append-only and keyed by number; superseded entries are marked, not deleted.

Format for each entry: **Status · Date · Task(s)**, then *Context → Decision → Consequences*. Every ADR names at least one cost. ADRs in this file carry status `Proposed` until the implementing task ships; they flip to `Accepted` with the implementing task ID at that point.

---

## ADR-001 — Push-based discovery via context injection, not pull-only via search

**Status:** Proposed · 2026-05 · [T20260510-11]

**Context.** Three classes of discovery were on the table:

| Approach | Profile |
|----------|---------|
| **Pull-only via search tool** | An `orbit.learning.search` MCP tool. Agents query when they think to. Lowest implementation cost; depends entirely on agent discipline. |
| **Push at session start** | All learnings (or an agent-curated subset) load into agent context at session start, like `CLAUDE.md` does. No discipline required, but unscoped and noisy at scale. |
| **Push at the moment of action** | Scoped injection triggered by the file path or task an agent is about to touch. Higher implementation cost; matches discoverability cost to relevance value. |

The repeated failure mode the system exists to prevent is *agents not knowing they should look*. Pull-only inherits that failure mode wholesale: the agent that needed the learning most — the one that forgot the rule — is the one who won't think to query. Session-start push avoids the discipline problem but punishes every session with content that may not apply.

**Decision.** Phase 1 ships push-at-the-moment-of-action across three layers: engine pre-prompt injection (universal, task-scoped), MCP tool-response sidecar (cross-agent, file-path-scoped), and Claude Code `PreToolUse` hook (Claude Code only, edit-scoped). A pull surface (`orbit.learning.search`, `orbit-learnings` skill) ships alongside as a complement, not a substitute.

**Consequences.**
- Agents get relevant learnings without having to query — the discoverability failure mode is closed.
- Authoring effort produces compounding value: every learning is delivered the next time anyone touches the relevant area, automatically.
- The three-layer architecture means coverage degrades gracefully: agents without hook support still get layers 1 and 2.
- Cost: every Orbit-spawned task and every relevant MCP tool call pays a small latency hit for the scope-match query, plus a few dozen tokens of context per injected learning. At expected scale (low hundreds of learnings, sub-millisecond match) the latency is negligible; the context cost is bounded by the per-call cap of 5 and the per-session cap of 20. The cost is real and paid uniformly — even on tasks where no learning applies, the engine still queries to find that out.

---

## ADR-002 — Native Orbit primitive (`learning` resource) over a flat markdown directory

**Status:** Accepted · 2026-05 · [T20260510-11] · [T20260511-5]

**Context.** Storage choice. Three plausible shapes:

1. **Flat markdown directory.** `docs/learnings/*.md` plus an index file. Easy to author with any text editor. Cheap to grep. Hard to query programmatically (no structured fields), hard to scope (path globs in markdown frontmatter are non-standard), no native lifecycle (supersession, staleness).
2. **Native primitive in `orbit-store`.** YAML on disk + SQLite index, mirroring tasks. Structured fields (`scope`, `evidence`, `status`), atomic mutations via `orbit.learning.*` tools, indexable for sub-10ms lookups. Implementation cost is real but reuses the existing layered store pattern.
3. **Hybrid: markdown bodies + YAML metadata.** Markdown for content, YAML frontmatter for structure. Familiar to many tools. Splits concerns awkwardly when programmatic mutations write to one half and humans edit the other.

The injection layers ([2_design.md §4](./2_design.md)) are the forcing function. Layer 1 has to query "which learnings match this task's context_files" before agent spawn; layer 2 has to do the same per MCP call. Both are hot paths. Grepping markdown frontmatter on every spawn or every tool call is the wrong shape — it makes every layer pay a full filesystem walk for what should be an indexed lookup.

A flat-markdown approach can be retrofitted with an index, but at that point it's a native primitive with extra steps and a less convenient on-disk format.

**Decision.** Phase 1 implements `learning` as a first-class Orbit resource: YAML records under `.orbit/learnings/<id>.yaml`, SQLite index under `learnings_index`, MCP/CLI surface mirroring `orbit.task.*`. Tasks were the model because they're the closest existing primitive in shape and lifecycle.

**Consequences.**
- Hot-path queries are indexed, sub-10ms, and don't pay filesystem-walk cost.
- Lifecycle (`status`, `supersedes`, `superseded_by`) is structurally enforceable.
- The CLI/MCP surface is symmetric with tasks, which lowers the cognitive cost for agents and humans who already know the task model.
- Cost: real implementation work — a new `orbit-store/file/learning_store/` module, a new SQLite table, six MCP tools, six CLI subcommands. This is non-trivial vs. "create a folder and grep it." The bet is that hot-path query performance and lifecycle enforcement justify the build cost over the lifetime of the system.

---

## ADR-003 — Workspace-scoped, checked into git (not workspace-private state)

**Status:** Accepted · 2026-05 · [T20260510-11] · [T20260511-5]

**Context.** Where do learning records live on disk?

- **Workspace state** (`.orbit/state/learnings/`, gitignored). Same locality as job runs, command audit, etc. Workspace-private; doesn't survive collaborator handoff.
- **Workspace-scoped, checked in** (`.orbit/learnings/<id>.yaml`, in git). Same locality as tasks. Travels with the repo across machines and collaborators.
- **Global** (`~/.orbit/learnings/`). Like the global skills location. Cross-workspace; requires conflict semantics if multiple workspaces author overlapping records.

Per the Scoping Rules table in [CLAUDE.md](../../../CLAUDE.md), tasks are `WorkspaceOnly` and live in `.orbit/tasks/` checked in. Job runs are also `WorkspaceOnly` but under `.orbit/state/`, gitignored, because they're execution artifacts. Learnings sit closer to tasks in shape — durable project artifacts authored over time — so the task locality is the right precedent.

The cross-workspace case ([3_vision.md §1.4](./3_vision.md)) is real but secondary: most learnings are repo-specific, and the cross-cutting ones are best handled by tag-driven promotion later, not by making the default storage location global.

**Decision.** Phase 1 stores learnings at `.orbit/learnings/<id>.yaml`, scoped `WorkspaceOnly` per the Scoping Rules table, checked into git. The SQLite index lives under `.orbit/state/` and is rebuildable from the YAML; it does not need to be checked in.

**Consequences.**
- Learnings travel with the repo. New collaborator clones, gets all the project knowledge from day zero.
- A learning authored on one machine and a task fix on another arrive in the same PR and review together, which keeps the knowledge in lockstep with the code that produced it.
- The git semantics for tasks (review, merge, conflict resolution) apply uniformly; no new mental model needed.
- Cost: every learning is a commit. PR diffs include learning records, which is fine for substantive learnings but adds review noise for housekeeping edits (typo fixes, scope-glob tweaks). Merge conflicts on the SQLite index are avoided by gitignoring it, but conflicts on the YAML are possible when two PRs add learnings simultaneously — handled by ID allocation (date + sequence), but worth noting.

---

## ADR-004 — Phase-1 scope = path globs + tags, ranked by recency; semantic and symbol-aware deferred

**Status:** Proposed · 2026-05 · [T20260510-11]

**Context.** A learning's scope (when does it match?) and ranking (which match wins?) have multiple plausible designs:

| Scope axis | Profile |
|------------|---------|
| **Path globs** | Match against file paths the agent is about to touch. Stable shape, simple matcher (reuses `orbit-policy`'s glob engine). Brittle to file renames. |
| **Tags** | Free-form labels. Survive renames. Require the author to anticipate the categorization. |
| **Symbol IDs** | Match against knowledge-graph symbols. Survive renames cleanly. Couples to graph rebuilds. |
| **Semantic similarity** | Match by embedding distance to current edit context. Catches relevance the other axes miss. Depends on semantic-search infrastructure. |

| Ranking | Profile |
|---------|---------|
| **Recency (`updated_at` desc)** | Trivial. Wrong when an old, important learning loses to a recent, marginal one. |
| **Manual `priority`** | Author-supplied. Honest signal when used; degenerates to "everything is high priority" without curation discipline. |
| **Semantic similarity** | Best signal. Requires embeddings. Cost = embed every learning + run cosine on every query. |

Phase 1's binding constraint is: ship before semantic-search reaches Accepted ([T20260510-3]). That rules out semantic similarity for both scope and ranking. Symbol-aware scope is *technically* available — the knowledge graph already exists — but coupling the learning store to graph rebuilds adds dependency surface and mainly pays off when fused with semantic ranking. Doing one without the other yields a clunky middle state.

**Decision.** Phase 1 supports two scope axes, evaluated as logical OR: path globs (matched via the `orbit-policy` glob engine) and tags (matched as exact strings). Ranking is `updated_at` desc with optional `priority` tagging as a tie-breaker. The schema reserves `scope.symbols` and `scope.semantic_seed` fields for phase 2 forward compatibility, but neither is read in phase 1.

Phase 2 ([3_vision.md §1.1](./3_vision.md), [§1.2](./3_vision.md)) layers symbol-aware scope and semantic ranking once semantic-search ships.

**Consequences.**
- Phase 1 is implementable in parallel with semantic-search work, not gated on it.
- Path globs cover the common case (most learnings are file-area-scoped) and tags cover the cross-cutting case.
- The schema is forward-compatible; phase 2 is additive, not a migration.
- Cost: recency-only ranking has known failure modes ([3_vision.md §1.2](./3_vision.md)) — old-but-important learnings get out-ranked by recent-but-marginal ones. Path globs are brittle to renames; the documented mitigation is "run `orbit learning prune --stale-only` after refactors that move files," which is operational discipline, not automation. Both costs are accepted as the price of shipping phase 1 ahead of semantic-search.

---

## ADR-005 — Three-layer push pipeline (engine pre-prompt + MCP sidecar + Claude Code hook), not single-layer

**Status:** Proposed · 2026-05 · [T20260510-11]

**Context.** The push-injection layer ([2_design.md §4](./2_design.md)) has multiple natural placements, each with different coverage:

- **Engine pre-prompt only.** Inject when `orbit-engine` spawns an agent for a task. Universal across agents. Coarse: fires once at task start, before the agent has read its way to the relevant code, so narrow learnings (file-path-scoped) may not surface for the file the agent edits ten tool calls in.
- **MCP-sidecar only.** Attach `learnings` to MCP tool responses that reference paths. Cross-agent. Misses Claude Code's built-in `Edit | Write | Read`, which agents use far more than they call MCP file tools.
- **Claude Code `PreToolUse` only.** Per-edit precision. Vendor-locked: doesn't apply to Codex, Gemini, Anthropic-API, Ollama, or any other agent runtime.
- **All three layered.** Each layer adds precision on top of the layers below. Coverage degrades gracefully: agents without hook support still get layers 1 and 2; tools without path arguments still get layer 1.

The vendor-locked single-layer options are non-starters because the project supports multiple agent providers (see `crates/orbit-agent/providers/`). Engine-pre-prompt-only misses the long-task case where an agent works for an hour through a wide context. MCP-sidecar-only misses the most-frequent agent action (built-in editor tools).

**Decision.** Phase 1 ships all three layers active simultaneously. Each layer consults a per-session deduplication set so the same learning doesn't inject multiple times across layers. Per-call cap of 5 learnings; per-session cap of 20.

**Consequences.**
- Coverage is robust: even if one layer misfires or a vendor lacks hook support, the others provide a baseline.
- Agents see relevant learnings at multiple natural moments — task start, MCP tool call, individual edit — without being drowned in repeats (dedup set).
- The architecture admits a future "layer 4" (Orbit-side proxy for agents without hooks) without restructuring, but doesn't require it ([3_vision.md §1.5](./3_vision.md)).
- Cost: three injection sites means three places to maintain. A schema change to learning records (new field surfaced at injection time) requires touching `orbit-engine`, `orbit-mcp`, and the Claude Code hook script. The dedup set is agent-local; if context is compressed mid-session, the set may reset and the same learning may inject twice. Both costs are accepted as the price of robust coverage; collapsing to a single layer would mean choosing one failure mode (vendor lock-in, coarse scope, or missing built-in tools) and living with it.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
