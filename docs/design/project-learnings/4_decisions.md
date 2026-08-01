---
summary: "Project Learnings — Decisions"
type: design
title: "Project Learnings — Decisions"
owner: claude
last_updated: 2026-08-01
status: Draft
feature: project-learnings
doc_role: decisions
tags: ["project-learnings"]
---

# Project Learnings — Decisions

ADR-style log of non-obvious project-learnings decisions. Each entry names the pressure, the choice, and the tradeoff. Entries are keyed by global ADR ID and ordered ascending. New entries are allocated via `orbit.adr.add` *before* the local heading is written — see [../CONVENTIONS.md §4](../CONVENTIONS.md) and the `orbit-knowledge` skill.

Format for each entry: **Status · Date · Task(s) · legacy_id (if backfilled)**, then *Context → Decision → Consequences*. Every ADR names at least one cost.

Historical note: entries below were originally numbered ADR-001 through ADR-006 within this folder. ADR-001 through ADR-005 were imported into the global store on 2026-05-11 (`ADR-0108`–`ADR-0112`) with `legacy_ids` set; ADR-006 was added directly to this file by [ORB-00095] without a global allocation and was backfilled as `ADR-0157` per [ORB-00098]. Each heading now carries the global ID; the original local IDs survive as `legacy_ids` so prior citations still resolve via `orbit.adr.list --legacy-id=project-learnings/ADR-NNN`.

Historical note ([ORB-10479]): the entries listed below already held a global ADR allocation, but their store bodies were lost when the worktrees that authored them were reaped (see [F2026-07-163]). The narratives were restored into the store at their existing IDs — no ID was reallocated — and their headings reduced to pointer form. Restored here: [ADR-0157].

---

## ADR-0108 — Push-based discovery via context injection, not pull-only via search (superseded)

**Status:** Superseded · 2026-07 · [ORB-10346] · legacy_id: `project-learnings/ADR-001`

**Supersession note.** Automatic delivery was retired after the 2026-07-18 relevancy audit. The current model is pull discovery through search/show plus concise reference comments; this entry remains as the historical decision it replaced.

**Context.** Three classes of discovery were on the table:

| Approach | Profile |
|----------|---------|
| **Pull-only via search tool** | An `orbit.search` MCP tool (with `kind: "learning"`). Agents query when they think to. Lowest implementation cost; depends entirely on agent discipline. |
| **Push at session start** | All learnings (or an agent-curated subset) load into agent context at session start, like `CLAUDE.md` does. No discipline required, but unscoped and noisy at scale. |
| **Push at the moment of action** | Scoped injection triggered by the file path or task an agent is about to touch. Higher implementation cost; matches discoverability cost to relevance value. |

The repeated failure mode the system exists to prevent is *agents not knowing they should look*. Pull-only inherits that failure mode wholesale: the agent that needed the learning most — the one that forgot the rule — is the one who won't think to query. Session-start push avoids the discipline problem but punishes every session with content that may not apply.

**Decision.** Phase 1 ships push-at-the-moment-of-action across three layers: engine pre-prompt injection (universal, task-scoped), MCP tool-response sidecar (cross-agent, file-path-scoped), and Claude Code `PreToolUse` hook (Claude Code only, edit-scoped). A pull surface (`orbit.search` with `kind: "learning"`, `orbit-learnings` skill) ships alongside as a complement, not a substitute.

**Consequences.**
- Agents get relevant learnings without having to query — the discoverability failure mode is closed.
- Authoring effort produces compounding value: every learning is delivered the next time anyone touches the relevant area, automatically.
- The three-layer architecture means coverage degrades gracefully: agents without hook support still get layers 1 and 2.
- Cost: every Orbit-spawned task and every relevant MCP tool call pays a small latency hit for the scope-match query, plus a few dozen tokens of context per injected learning. At expected scale (low hundreds of learnings, sub-millisecond match) the latency is negligible; the context cost is bounded by the per-call cap of 5 and the per-session cap of 20. The cost is real and paid uniformly — even on tasks where no learning applies, the engine still queries to find that out.

---

## ADR-0109 — Native Orbit primitive (`learning` resource) over a flat markdown directory

**Status:** Accepted · 2026-05 · [T20260510-11] · [T20260511-5] · legacy_id: `project-learnings/ADR-002`

**Context.** Storage choice. Three plausible shapes:

1. **Flat markdown directory.** `docs/learnings/*.md` plus an index file. Easy to author with any text editor. Cheap to grep. Hard to query programmatically (no structured fields), hard to scope (path globs in markdown frontmatter are non-standard), no native lifecycle (supersession, staleness).
2. **Native primitive in `orbit-store`.** YAML on disk + SQLite index, mirroring tasks. Structured fields (`scope`, `evidence`, `status`), atomic mutations via `orbit.learning.*` tools, indexable for sub-10ms lookups. Implementation cost is real but reuses the existing layered store pattern.
3. **Hybrid: markdown bodies + YAML metadata.** Markdown for content, YAML frontmatter for structure. Familiar to many tools. Splits concerns awkwardly when programmatic mutations write to one half and humans edit the other.

The injection layers ([2_design.md §4](./2_design.md)) are the forcing function. Layer 1 has to query "which learnings match this task's context_files" before agent spawn; layer 2 has to do the same per MCP call. Both are hot paths. Grepping markdown frontmatter on every spawn or every tool call is the wrong shape — it makes every layer pay a full filesystem walk for what should be an indexed lookup.

A flat-markdown approach can be retrofitted with an index, but at that point it's a native primitive with extra steps and a less convenient on-disk format.

**Decision.** Phase 1 implements `learning` as a first-class Orbit resource: YAML records under `.orbit/learnings/<id>/learning.yaml`, SQLite index under `learnings_index`, MCP/CLI surface mirroring `orbit.task.*`. Tasks were the model because they're the closest existing primitive in shape and lifecycle.

**Consequences.**
- Hot-path queries are indexed, sub-10ms, and don't pay filesystem-walk cost.
- Lifecycle (`status`, `supersedes`, `superseded_by`) is structurally enforceable.
- The CLI/MCP surface is symmetric with tasks, which lowers the cognitive cost for agents and humans who already know the task model.
- Cost: real implementation work — a new `orbit-store/file/learning_store/` module, a new SQLite table, six MCP tools, six CLI subcommands. This is non-trivial vs. "create a folder and grep it." The bet is that hot-path query performance and lifecycle enforcement justify the build cost over the lifetime of the system.

---

## ADR-0110 — Workspace-scoped, checked into git (not workspace-private state)

**Status:** Accepted · 2026-07 · [T20260510-11] · [T20260511-5] · [ORB-10452] · legacy_id: `project-learnings/ADR-003`

**Context.** Where do learning records live on disk?

- **Workspace state** (`.orbit/state/learnings/`, gitignored). Same locality as job runs, command audit, etc. Workspace-private; doesn't survive collaborator handoff.
- **Workspace-scoped, checked in** (`.orbit/learnings/<id>/learning.yaml`, in git). Same locality as tasks. Travels with the repo across machines and collaborators.
- **Global** (`~/.orbit/learnings/`). Like the global skills location. Cross-workspace; requires conflict semantics if multiple workspaces author overlapping records.

Per the Scoping Rules table in [CLAUDE.md](../../../CLAUDE.md), tasks are `WorkspaceOnly` and live in `.orbit/tasks/` checked in. Job runs are also `WorkspaceOnly` but under `.orbit/state/`, gitignored, because they're execution artifacts. Learnings sit closer to tasks in shape — durable project artifacts authored over time — so the task locality is the right precedent.

The cross-workspace case ([3_vision.md §1.4](./3_vision.md)) is real but secondary: most learnings are repo-specific, and the cross-cutting ones are best handled by tag-driven promotion later, not by making the default storage location global.

**Decision.** Phase 1 stores learnings at `.orbit/learnings/<id>/learning.yaml`, scoped `WorkspaceOnly` per the Scoping Rules table, checked into git. The SQLite index lives under `.orbit/state/` and is rebuildable from the YAML; it does not need to be checked in.

**Amendment — ORB-00096.** Learnings moved from the original flat `.orbit/learnings/<id>.yaml` / `.orbit/learnings/superseded/<id>.yaml` layout to per-entity directories at `.orbit/learnings/<id>/learning.yaml`. Status now lives only in the YAML body, and the explicit `orbit learning migrate-layout` command performs the one-way migration.

**Amendment — ORB-10452.** The one-way migration is now report-only on a bare invocation and applies only with the standard non-interactive `--confirm` flag. The explicit gate preserves scriptability without an stdin prompt and makes the irreversible layout operation follow the CLI-wide destructive-command convention.

**Consequences.**
- Learnings travel with the repo. New collaborator clones, gets all the project knowledge from day zero.
- A learning authored on one machine and a task fix on another arrive in the same PR and review together, which keeps the knowledge in lockstep with the code that produced it.
- The git semantics for tasks (review, merge, conflict resolution) apply uniformly; no new mental model needed.
- Cost: every learning is a commit. PR diffs include learning records, which is fine for substantive learnings but adds review noise for housekeeping edits (typo fixes, scope-glob tweaks). Merge conflicts on the SQLite index are avoided by gitignoring it, but conflicts on the YAML are possible when two PRs add learnings simultaneously — handled by ID allocation (date + sequence), but worth noting.

---

## ADR-0111 — Phase-1 scope = path globs + tags; semantic and symbol-aware deferred

**Status:** Accepted · 2026-05 · [T20260511-6] · legacy_id: `project-learnings/ADR-004`

**Context.** A learning's scope (when does it match?) and ranking (which match wins?) have multiple plausible designs:

| Scope axis | Profile |
|------------|---------|
| **Path globs** | Match against file paths the agent is about to touch. Stable shape, simple matcher (reuses `orbit-policy`'s glob engine). Brittle to file renames. |
| **Tags** | Free-form labels. Survive renames. Require the author to anticipate the categorization. |
| **Symbol IDs** | Match against knowledge-graph symbols. Survive renames cleanly. Couples to graph rebuilds. |
| **Semantic similarity** | Match by embedding distance to current edit context. Catches relevance the other axes miss. Depends on orbit-search infrastructure. |

| Ranking | Profile |
|---------|---------|
| **Recency (`updated_at` desc)** | Trivial. Wrong when an old, important learning loses to a recent, marginal one. Superseded as the primary ranking key by [ADR-0157]. |
| **Manual `priority`** | Author-supplied. Honest signal when used; degenerates to "everything is high priority" without curation discipline. |
| **Semantic similarity** | Best signal. Requires embeddings. Cost = embed every learning + run cosine on every query. |

Phase 1's binding constraint was to ship before orbit-search reached Accepted
([T20260510-3]). That ruled out semantic similarity for both scope and ranking.
At the time, symbol-aware scope was technically available through the code
graph, but coupling the learning store to rebuilds added dependency surface and
mainly paid off when fused with semantic ranking. ADR-0291 / ORB-10491 later
removed that subsystem, so the reserved symbol field has no live resolver.

**Decision.** Phase 1 supports two scope axes, evaluated as logical OR: path globs (matched via the `orbit-policy` glob engine) and tags (matched as exact strings). The schema reserves `scope.symbols` and `scope.semantic_seed` fields for phase 2 forward compatibility, but neither is read in phase 1. Initial ranking used `updated_at` desc with optional `priority`; [ADR-0157] adds decay-weighted upvotes ahead of those tie-breakers.

Phase 2 ([3_vision.md §1.1](./3_vision.md), [§1.2](./3_vision.md)) layers symbol-aware scope and semantic ranking once orbit-search ships.

**Consequences.**
- Phase 1 is implementable in parallel with orbit-search work, not gated on it.
- Path globs cover the common case (most learnings are file-area-scoped) and tags cover the cross-cutting case.
- The schema is forward-compatible; phase 2 is additive, not a migration.
- Cost: path globs are brittle to renames; the documented mitigation is "run `orbit learning prune --stale-only` after refactors that move files," which is operational discipline, not automation. Ranking still lacks semantic similarity until phase 2, even after [ADR-0157]'s vote signal.

---

## ADR-0112 — Three-layer push pipeline (engine pre-prompt + MCP sidecar + Claude Code hook), not single-layer (superseded)

**Status:** Superseded · 2026-07 · [ORB-10346] · legacy_id: `project-learnings/ADR-005`

**Supersession note.** The three automatic-delivery layers are no longer active. [ORB-10346] removes the repository hook registrations and retains explicit search/show retrieval with point-of-use reference comments.

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
- Cost: three injection sites means three places to maintain. A schema change to learning records (new field surfaced at injection time) requires touching `orbit-engine`, `orbit-remote`, and the Claude Code hook script. The dedup set is agent-local; if context is compressed mid-session, the set may reset and the same learning may inject twice. Both costs are accepted as the price of robust coverage; collapsing to a single layer would mean choosing one failure mode (vendor lock-in, coarse scope, or missing built-in tools) and living with it.

---

## ADR-0157 — Rank matched learnings by task-anchored decay-weighted upvotes

**Status:** Superseded by ADR-0210 · 2026-05 · [ORB-00095] · legacy_id: `project-learnings/ADR-006`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0157"}'`.

---

## ADR-0210 — Remove the learning vote and comment surfaces

**Status:** Accepted · 2026-07 · [ORB-10046]

**Context.** Two auxiliary surfaces shipped beside the core learning primitives: task-anchored decayed upvotes (ADR-0157) and free-text comments anchored to a learning. After months of availability, orbit's own 50-record learning corpus contained zero votes and exactly one comment (L-0005). The infrastructure carrying those surfaces was disproportionate to their use: two JSONL sidecars per learning, a decayed-score search-ranking pass, a `learning_votes_received` scoreboard column, an `orbit.learning.comment.add` entry in the artifact-redaction policy (with its own audit path), a `LearningReminder.comments` field hydrated on every reminder render, and the CLI/MCP tool surfaces themselves. ORB-00289/00348 had already flipped `upvote`, `comment.list`, and `comment.delete` to `register_inactive`; this decision finishes the direction.

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Keep both, narrower** | Preserve `comment.add` as an annotation channel and `upvote` for recency, with clearer docs on when to prefer `update`/`supersede`. |
| **Remove comments, keep upvote** | Halves the removal but keeps the sidecar and the decayed-score ranking; still no observed votes to justify. |
| **Remove both (this ADR)** | Corrections funnel through `update`, material changes through `supersede`, provenance through `evidence`; ranking reduces to `priority` → `updated_at` → `id`. |

**Decision.** Remove both surfaces entirely. Delete `orbit.learning.{upvote,comment.add,comment.list,comment.delete}`, the CLI `orbit learning upvote` / `comment` subcommands, the store trait methods (`upvote_learning`, `learning_vote_summary`, `add_learning_comment`, `list_learning_comments`, `delete_learning_comment`), the `votes.jsonl`/`comments.jsonl` sidecars and their layout/validation helpers, the `learning_votes_received` scoreboard column, the `LearningComment{,Event,Tombstone}` / `LearningVote{Row,Summary}` / `decayed_vote_score` / `read_comment_render_cap_env` / `DEFAULT_LEARNING_COMMENT_RENDER_CAP` types, the `NotFoundKind::LearningComment` variant, the `LearningCommentAdd` artifact-redaction policy entry, and the `LearningReminder.comments` field. Preserve the one existing comment (L-0005/C20260519-1) by folding its content into that learning's body via `orbit.learning.update` before deleting the surface.

**Consequences.**
- Search ranking becomes `priority` desc → `updated_at` desc → `id` asc. Since no learning had votes, this changes no observed rankings today.
- The free-text redaction burden shrinks (the `comment.add` policy entry, its `learning_comment` artifact-target mapping, and the audit-emit branch are gone).
- The store layout simplifies: each learning is `<L-id>/learning.yaml` and nothing else in the common case. `sync`/reindex no longer validates comment JSONL. Partial-create rollback no longer removes sidecar files.
- `LearningReminder` shrinks; consumers that hand-constructed reminders with `comments: []` no longer compile.
- Duplicate-check regains its original character: "this already exists" is a signal to the human curator, not a mechanical vote against the duplicate. If a stronger signal is needed later, it can be added with usage data behind it.
- Cost: breaking change to the MCP/CLI/store trait surface; the `EXPECTED_INACTIVE_TOOL_NAMES` length canary moves from 26 to 22. External callers of the removed tools or types must migrate to `update`/`supersede`/`evidence`. Documented under Breaking Changes in `CHANGELOG.md` [ORB-10046].
- Cost: the ranking signal ADR-0157 was designed for — repeated task-anchored validation of an older but still load-bearing learning — is not replaced. `updated_at` (bumped on every `update`) is the natural proxy; if evidence emerges that it's insufficient, a scoped feedback primitive can be reintroduced with real usage data behind it.

---

## ADR-0212 — Workspace-scope the learning envelope index in the shared host-global database

**Status:** Accepted · 2026-07 · [ORB-10113]

**Context.** The learning envelope index lives in the host-global `~/.orbit/orbit.db`, but `learnings_index` had no workspace discriminator and was keyed only by learning ID. Each workspace runtime pairs that shared table with its own workspace-local `.orbit/learnings/` YAML root, and `sync_learnings` truncated the entire table before reinserting one workspace's records. During the multi-workspace ship sweep on `dk1`, runtimes searched index rows written by *other* workspaces, emitting repeated `skipping learning reminder because the YAML body is missing … learning not found: L-0002` warnings during reminder hydration. This is more than log noise: several legacy workspaces hold different records under the same canonical-looking ID (e.g. `L-0002`), so when the foreign ID also exists locally the existence check passes and Orbit can silently inject the foreign indexed summary under the local learning ID. This directly contradicts the design's §5.3 framing ("no shared store across workspaces") — the store *is* shared; only the YAML was ever partitioned.

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Attribute legacy rows to a workspace during migration** | Impossible to do reliably — the pre-scoping table records no owner, and duplicate IDs across workspaces make any guess a potential silent-injection vector. |
| **Move the index into each workspace's `.orbit/state/`** | Ends sharing entirely, but fragments the single host-global store the rest of the SQLite state (`job_runs`, `v2_audit_events`) already lives in, and duplicates open/migration cost per workspace. |
| **Composite `(workspace_id, id)` key (this ADR)** | Keeps one shared table but partitions rows by the stable registered workspace ID, matching how `job_runs`/`v2_audit_events` already scope. |

**Decision.** Re-key `learnings_index` by the composite `(workspace_id, id)`, where `workspace_id` is the **stable registered Orbit workspace ID** read from `.orbit/config.yaml` (the same id used for `job_runs` and `v2_audit_events`), not a path-derived label that changes across worktrees. Every index operation — search, upsert, delete, truncate/rebuild, cache materialization, and reminder lookup — is filtered by the runtime's own `workspace_id`; `LearningFileStore` carries the id and passes it to each `Store` index method. A v2 schema migration (`learnings_index_workspace_scope`) drops and recreates the table with the composite key. Because YAML is the source of truth and legacy rows cannot be attributed to a workspace, the migration **discards every envelope row** and lets each runtime rebuild its own via `sync`. The migration touches only SQLite and never reads or modifies any `learning.yaml`.

**Consequences.**
- Syncing workspace A can no longer read, truncate, or overwrite workspace B's rows; sequential and concurrent multi-workspace sweeps over the shared database are safe.
- The reminder defensive local-body check (`get_learning` before injecting a summary) still skips genuine *same-workspace* index ghosts, which is now the only class of ghost it can see.
- Cost: existing envelope rows are discarded on upgrade and rebuilt from YAML on each workspace's next `sync`. The documented rollback is the same operation — rerun `orbit learning sync` per workspace; discarded legacy rows are intentionally not restored.
- Cost: the design doc §2.2 schema and §5.3 sharing statement were stale and are corrected in the same change; any external reader of `learnings_index` must now include `workspace_id` in its predicates.

---

## ADR-0242 — Teaser learning injection + show-as-usage-signal (replaces rejected ack design)

**Status:** Accepted · 2026-07 · [ORB-10316]

**Context.** The 2026-07-18 relevancy audit (friction F2026-07-092) found the learning PreToolUse hook fired 2,374 times over two weeks with 13 injections (0.55%) and **zero usage signal**: nothing recorded whether an injected learning shaped the receiving agent's work, so nothing could drive deprecation of stale learnings. ADR-0210 removed the vote/comment feedback surfaces for lack of real usage, with an explicit reopening clause: a scoped feedback primitive can return "with real usage data behind it." A first attempt (PR #657, closed unmerged) added an explicit `orbit learning ack` CLI/MCP surface — Daniel rejected it as gameable, agent-remembered, and a new tool in the frozen conformance fixture, with "unacked = ignored" penalizing every silent session. Separately, per-session dedup was dead: `ORBIT_SESSION_ID` was exported on 0/2,374 fires and the ppid-tmpfile fallback re-keys per invocation (L-0077 injected 10× in one session).

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Explicit `learning ack` surface (PR #657)** | Active, gameable, adds an MCP tool to the frozen conformance fixture and a reminder footer line; silence forced to mean "ignored." Rejected. |
| **Full-content injection, no signal (status quo)** | High per-fire token cost, no usage data, no deprecation input. |
| **Teaser injection + show-as-signal (this ADR)** | Injection carries only id + summary + tags; opening the body via `orbit learning show` is the passive, ungameable signal. Lower token cost, no new agent action, no new MCP tool. |

**Decision.** Injection projects only the learning id, one-line summary, and scope tags; the full body is retrieved via `orbit learning show <id>`, which records a `learning_shown` audit event (keyed by learning id + session) in the host-global `~/.orbit/orbit.db` — the passive usage signal. `orbit learning stats` folds `learning_injected` + `learning_shown` into a per-learning rollup (injected, shown, shown ratio, last-injected/last-shown). Both emissions **fail open**: an unavailable audit backend logs a warning and the injection/show still completes. Session dedup keys on the first resolvable anchor: `ORBIT_SESSION_ID` env → the `session_id` field the hook payload carries → ppid-tmpfile last resort. **No ack surface** — no `orbit learning ack`, no `orbit.learning.ack`, no ack instruction in the injected block.

**Amendment — ORB-10346.** The injection half of this decision is retired: automatic delivery stopped on 2026-07-20 and its counters are frozen historical calibration. `orbit learning show`, its `learning_shown` audit event, and the `orbit learning stats` rollup remain active; a zero-new-injections future is valid.

**Consequences.**
- The rollup is the designed input for downstream deprecation policy (ORB-10318); decay/TTL is deliberately follow-up work, not implemented at this layer.
- The `learning_injected`/`learning_shown` contract lives in audit-event conventions (`target_type` + `arguments_json`), enforced by store-level fold tests, not a schema migration.
- Signal quality depends on agents opening learnings they use, but `show` is far harder to game than an ack and costs nothing extra to emit; the ratio is directional input for a sweep, not an automated gate.
- Cost: one audit row per `show`, plus scope tags on each teaser line. **No** change to the MCP conformance surface (no new tool), unlike the rejected ack design.

---

## ADR-0248 — Keep `orbit hook install` opt-in; remove `--hooks` from `workspace init`

**Status:** Accepted · 2026-07 · [ORB-10366]

**Context.** [ORB-10346] removed the learning-reminder registrations this repo's own `.claude/settings.json` and `.codex/config.toml` carried, but deliberately left the writer mechanism untouched: `orbit workspace init --hooks` and `orbit hook install` both still silently wrote those registrations back for any repo that invoked them, and the tree still carried two now-inert tracked shim files left over from before the retirement.

**Decision.** Remove the `--hooks` flag from `orbit workspace init` entirely — clap now rejects it as an unknown argument instead of silently ignoring it — and delete the tracked inert shim files. Keep `orbit hook install` / `orbit hook uninstall` as an explicit, opt-in, human-invoked escape hatch. The distinction is *automatic* vs. *deliberate*: ADR-0108/ADR-0112's failure mode was learnings pushed into context without anyone choosing that. A `--hooks` flag riding along on `workspace init` (run for unrelated reasons — bootstrapping a checkout) reproduces exactly that. `orbit hook install` has no such ambiguity — it is the only thing the command does, so running it is itself the deliberate opt-in choice.

**Consequences.**
- No code path reachable from `orbit init` or `orbit workspace init` writes a learning-reminder registration; a fresh `workspace init` against a temp workspace root leaves `.claude/settings.json` / `.codex/config.toml` untouched (`crates/orbit-cli/tests/hook_install.rs`).
- `orbit workspace init --hooks` now fails loudly (clap "unexpected argument") instead of silently succeeding.
- `orbit hook install` / `orbit hook uninstall`, the underlying `orbit_cmd::hook_install` module, the `orbit hook pretooluse` runtime path, and the independent `scripts/orbit-file-lock` PreToolUse guard are all unchanged.
- Cost: `orbit hook install` remains capable of re-registering the retired delivery mechanism if a human runs it deliberately — accepted as the intended behavior of an opt-in escape hatch, not a gap.
- Cost: the two tracked inert shim files this repo carried are deleted; `orbit hook install` regenerates them byte-identically if ever re-run, so nothing is lost.

---

## ADR-0250 — Gate learning authoring surfaces on caller role derived from the agent-identity env

**Status:** Accepted · 2026-07 · [ORB-10364]

**Context.** Policy since 2026-07-18: task executors file *frictions*; project learnings are authored by the orchestrator or by Daniel. The rule lived entirely in orchestrator-side prompt text and nothing on the box enforced it. Three different executors violated it in a single day's queue (friction F2026-07-102): L-0108 from a sol executor, L-0109 plus an update to L-0082 from an opus executor. The content was not the problem — the *surface* was. Because the learning store feeds scope injection and curation sweeps, unreviewed executor-authored entries accumulate as a side effect of ordinary task runs, and curation work ([ORB-10349] / [ORB-10362]) then spends real effort re-anchoring and narrowing exactly those entries. Three consecutive failures is evidence that prompt text is the wrong enforcement layer.

Alternatives considered:

| Approach | Profile |
|----------|---------|
| **Keep it in prompt text** | Zero code. Already failed three times in one day; the failure mode is silent and only visible during curation. |
| **Gate the store-level chokepoint** (`create_learning` / `update_learning` / `supersede_learning`) | One call site instead of six. But it also catches the dashboard's HTTP write actions, and [ORB-10352] had just moved dashboard write attribution *off* server env precisely because the dashboard can run with an ambient agent identity — gating there would block Daniel's own dashboard edits. Rejected. |
| **Gate the authoring surfaces** (this ADR) | The CLI subcommands and `orbit.learning.*` tools an executor can actually reach, via shared role-gated runtime wrappers. Six call sites, no collateral on the broker or fixtures. |

**Decision.** `orbit learning add` / `update` / `supersede` and their `orbit.learning.*` tool equivalents route through role-gated `OrbitRuntime::author_learning{,_update,_supersede}` wrappers that call `ensure_learning_write_allowed` (`crates/orbit-core/src/command/learning_authoring.rs`). The caller's role is derived from the agent-identity env pair the audit middleware already reads — `ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL`, assembled for every spawned run by orbit-engine's `provenance_env` builder — consumed through the existing `ActorIdentity::from_env`. **No parallel identity mechanism.** An executor-context caller is refused with `OrbitError::PolicyDenied` whose message names `orbit friction add` as the correct channel and echoes the attempted summary/body (truncated at 1500 chars) so the observation is not lost. `ORBIT_LEARNING_AUTHOR=1` is the sanctioned opt-in for an orchestrator dispatching curation work *as* an agent; the provenance builder never emits it, so it cannot be inherited by accident, and only the exact affirmative spellings `1` / `true` / `TRUE` count.

**Consequences.**
- Enforcement moves from prompt text into code at the two surfaces an executor can actually reach. The two entry points share one gate, so CLI and MCP cannot drift.
- Reads are untouched in every context: `show`, `list`, `search`, `stats`, and scope injection behave identically for human, executor, and opted-in orchestrator callers.
- `sync`, `prune`, `archive`, the multi-host owner-finalize path, and test fixtures keep the ungated store-level methods, which now carry doc comments pointing at the `author_*` wrappers.
- The dashboard needed one explicit change: `PATCH /api/learnings/:id` delegated to the `orbit.learning.update` tool (to inherit the superseded-record rejection), which would have put a human's dashboard edit behind a gate keyed on the *server process's* environment. The tool handler is now split into shared payload parsing plus the gated write, and the route calls a new ungated `OrbitRuntime::update_learning_from_request`. `POST .../supersede` already called the ungated runtime method and is unchanged.
- `finalize_preallocated_learning` stays ungated deliberately: by the time a caller reaches finalize the global id is already consumed and cannot be released, so a refusal there would burn an id. When [ORB-10274] (F3) routes public `orbit.learning.add` through the broker, the gate belongs in the preflight *before* `compose_preallocated_knowledge_add` allocates.
- `crates/orbit-cli/tests/learning.rs`, `crates/orbit-cli/tests/docs.rs`, and the tool-host learning tests now declare their caller context explicitly instead of inheriting whatever the suite was launched with. This gate turns the [ORB-10350] ambient-leak hazard from a latent inconsistency into a hard failure, so any fixture that seeds a learning has to say which context it means.
- Cost: the gate is applied at six call sites rather than one chokepoint, so a future authoring surface must remember to use the `author_*` wrappers — a tradeoff accepted to keep the dashboard's request-derived attribution working.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [ORB-10046] — Remove the vote and comment surfaces from the learning subsystem (ADR-0210 supersedes ADR-0157).
- [ORB-10316] — Teaser injection + `learning_shown` usage signal + `orbit learning stats` rollup + payload-derived session dedup (ADR-0242).
- [ORB-10346] — Retired automatic learning delivery while retaining pull discovery, `learning_shown`, and historical usage stats.
- [ORB-10366] — Removed the `--hooks` flag from `orbit workspace init` and the tracked inert shims; kept `orbit hook install` as an opt-in escape hatch (ADR-0248).
- [ORB-10364] — Gated the learning authoring surfaces on caller role and redirected executors to `friction add` (ADR-0250).
- [ORB-10452] — Made the legacy learning-layout migration report-only by default and require the standard `--confirm` apply flag (ADR-0110 amendment).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
