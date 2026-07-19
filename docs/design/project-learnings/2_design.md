---
summary: "Project Learnings — Design"
type: design
title: "Project Learnings — Design"
owner: claude
last_updated: 2026-07-19
status: Draft
feature: project-learnings
doc_role: design
tags: ["project-learnings"]
---

# Project Learnings — Design

This document specifies phase-1 project-learnings: the placement of learning storage in `orbit-store`, the schema of a learning record plus sidecars, the phase-1 scope-matching algorithm (path globs + tags), the three-layer push-injection pipeline (engine pre-prompt + MCP sidecar + optional Claude Code hook), the pull surface (skill + tools), the curation lifecycle, and the concerns the design deliberately leaves to follow-ups.

Phase 2 (semantic ranking, symbol-aware scope) is out of scope for this document and is captured in [3_vision.md §1.2](./3_vision.md). The schema in [§2](#2-learning-record-schema) is forward-compatible with phase 2.

---

## 1. Architectural Placement

Learnings live alongside tasks in the existing layered store. No new top-level crate is needed; the resource is structurally similar enough to a task that adding a parallel module preserves the project's "match existing patterns" rule from [CLAUDE.md](../../../CLAUDE.md).

```
orbit-store/
├── file/
│   ├── task_store/        # existing
│   └── learning_store/    # new — YAML + index, mirrors task_store
└── sqlite/
    └── learnings.rs       # new — index for fast scope-glob lookups
```

`orbit-tools` gains a `learning::` submodule that exposes `orbit.learning.add | list | search | show | update | supersede | upvote` as MCP tools. `orbit-cli` exposes the corresponding `orbit learning <subcommand>` shell surface.

`orbit-engine` gains the **pre-prompt injection** logic: before invoking an agent runtime for a task, it queries the learning store for entries whose `scope` matches the task's `context_files` and prepends formatted summaries to the agent prompt. This is the layer that makes push-based discovery cross-agent, because injection happens above the agent boundary ([§4](#4-push-injection-pipeline), [4_decisions.md ADR-005](./4_decisions.md)).

`orbit-remote` gains a thin MCP result decorator that, for tool responses referencing file paths, attaches a `learnings` sidecar field with up to N matching entries. This is the second push layer; it works for any agent that calls Orbit's MCP tools.

The third push layer — a Claude Code `PreToolUse` hook on `Edit | Write | Read` — is not part of any Orbit crate; it ships as a hook configuration in [.claude/settings.json](../../../.claude/settings.json) (or whichever scope is appropriate; see [§4.3](#43-layer-3-claude-code-pretooluse-hook-optional)).

No cross-crate dependencies that violate the architecture diagram in [CLAUDE.md](../../../CLAUDE.md) are introduced. The dependency edges added are `orbit-store` (extended internally), `orbit-tools → orbit-store` (already present), and `orbit-engine → orbit-store` (already present). `orbit-mcp` remains a generic transport kernel that depends only on `orbit-common`; `orbit-remote` owns Layer 2 composition and asks the injected host to query learning candidates instead of coupling the result decorator to learning persistence.

---

## 2. Learning Record Schema

### 2.1 On-disk format

Each learning owns a directory under `.orbit/learnings/<id>/`, mirroring the task bundle layout. The source-of-truth YAML lives at `.orbit/learnings/<id>/learning.yaml`; per-learning sidecars such as `votes.jsonl` live beside it without polluting the root:

```yaml
id: L-0001
schemaVersion: 1
status: active                    # active | superseded
created_at: 2026-05-09T18:00:00Z
updated_at: 2026-05-09T18:00:00Z
created_by: claude

scope:
  paths:
    - "crates/orbit-engine/**/perf*.rs"
    - "benchmarks/graph-latency/**"
  tags:
    - performance
    - benchmarking
  # phase 2 will add:
  # symbols: [...]
  # semantic_seed: "..."

summary: >
  Never declare a perf win on latency alone — verify output equivalence
  between old and new code paths before freezing a result.

body: |
  Latency improvements that change observable behavior are regressions
  dressed as wins. Before declaring any perf result, compare outputs of
  the old and new code paths on the same inputs and assert byte-for-byte
  equivalence (or document the diff and why it's acceptable).

  **Why:** A graph-latency v1 benchmark showed a 4× speedup that turned
  out to be the new path silently dropping symbols.

  **How to apply:** When working on `benchmarks/graph-latency/**` or any
  `perf*` module, the validation phase must include an equivalence check
  alongside the timing measurement.

evidence:
  - kind: task
    ref: T20260510-1
  - kind: task
    ref: T20260510-2
  - kind: commit
    ref: 3edf00ed

supersedes: null                  # set to L-id if this replaces an older entry
```

The legacy flat layout (`.orbit/learnings/<id>.yaml` plus `.orbit/learnings/superseded/<id>.yaml`) is rejected on load with an actionable migration error. `orbit learning migrate-layout` performs the explicit one-way move and leaves `tags.yaml` at `.orbit/learnings/tags.yaml`.

### 2.2 SQLite index

A SQLite table `learnings_index` mirrors a few columns for fast scope matching, since brute-forcing path globs over every YAML on every tool call is the wrong shape. The table lives in the host-global `orbit.db` and holds rows for every workspace bound to that database, so it is **partitioned by the stable registered workspace ID** — the composite `(workspace_id, id)` key (ADR-0212). Without the discriminator, a multi-workspace sweep over the shared database searched, truncated, or overwrote another workspace's rows:

```sql
CREATE TABLE learnings_index (
    workspace_id TEXT NOT NULL,           -- stable registered Orbit workspace id
    id           TEXT NOT NULL,           -- L-0001
    status       TEXT NOT NULL,           -- "active" | "superseded"
    paths        TEXT NOT NULL,           -- JSON array of glob patterns
    tags         TEXT NOT NULL,           -- JSON array of tags
    summary      TEXT NOT NULL,           -- denormalized for fast read
    updated_at   TEXT NOT NULL,
    priority     INTEGER,                 -- optional ranking key
    PRIMARY KEY (workspace_id, id)
);

CREATE INDEX learnings_active
    ON learnings_index(workspace_id, status) WHERE status = 'active';
```

Query path: filter to the runtime's own `workspace_id` and `status = 'active'`, load the small set of `(paths, tags)` rows, run the in-memory glob match. At expected scale (low hundreds of active learnings), this is sub-millisecond; the index exists to avoid YAML I/O on every tool call.

The YAML files are the source of truth. The index is rebuildable from them via `orbit learning sync`.

Vote rows are source-of-truth sidecars, not SQLite projections in v1. `orbit learning sync` still walks every per-learning `votes.jsonl` and fails on invalid JSONL, so cache rebuilds do not silently ignore corrupted vote files.

### 2.3 ID format

`L-NNNN` — same shape as task IDs, different prefix. Allocated by `orbit.learning.add`, never invented by agents (same rule as task IDs).

---

## 3. Scope Matching (phase 1)

Phase 1 supports two scope axes, evaluated as a logical OR:

### 3.1 Path globs

Glob patterns over repo-relative paths. Matched against any file path that:

- Appears in a task's `context_files` (engine pre-prompt path).
- Is referenced in an MCP tool argument or response (MCP-sidecar path).
- Is the target of `Edit | Write | Read` (Claude Code hook path).

Glob syntax: standard `**`/`*`/`?` semantics (the same matcher `orbit-policy` uses for `read`/`modify` rules — reused, not reimplemented). A learning matches if **any** of its `scope.paths` matches the candidate path.

### 3.2 Tags

Free-form string labels. Matched against:

- Tags on the task itself (when in the engine pre-prompt path).
- Tags supplied by the caller in an explicit `orbit learning list --tag` query (structural filter; post-[ORB-00202]).

Tags are not auto-derived from anything in phase 1. They exist for the cases where path-based scoping doesn't fit ("when running any benchmark", "when authoring docs").

### 3.3 Combination

A learning matches a candidate if **(path glob matches) OR (any tag matches)**. The OR is deliberate: the two axes capture different shapes of relevance and shouldn't gate each other.

### 3.4 Why not symbol-aware in phase 1

Symbol-aware scoping (e.g. "this learning applies whenever the agent touches the `cosine_similarity` function regardless of where it lives") is more precise than path globs but couples the learning store to the knowledge graph. Phase 2 picks this up alongside semantic ranking; phase 1's scope schema reserves a `scope.symbols` field for forward compatibility ([3_vision.md §1.1](./3_vision.md)).

---

## 4. Push-Injection Pipeline

Three layers, from coarsest to finest. Each layer adds precision on top of the layers below; all three may be active simultaneously, with deduplication described in [§4.4](#44-deduplication-and-budget).

### 4.1 Layer 1 — Engine pre-prompt injection (universal)

`orbit-engine` is the layer that spawns agents for tasks. Before the agent runtime starts, the engine:

1. Reads the task's `context_files`.
2. Reads the task's `tags` (if any).
3. Queries the runtime-side `search_learnings` helper (equivalent to `orbit.search` with `kind: "learning"`) with the union of (paths from `context_files`) and (tags from the task).
4. Takes the top-K (default 5) results.
5. Prepends a `<system-reminder>` block to the agent prompt:

   ```
   <system-reminder>
   Project learnings relevant to this task:

   - [L-0001] Never declare a perf win on latency alone — verify
     output equivalence before freezing a result.
   - [L-0014] When editing tree-sitter extractors, the …

   Read full body via `orbit.learning.show <id>` if needed.
   </system-reminder>
   ```

**Prerequisite.** The tag-matching half of step 3 depends on the `Task` schema carrying a `tags: Vec<String>` field, which does not exist today. That schema change is tracked separately as [T20260510-12] and is a hard prerequisite for this layer's tag axis. Path-glob matching against `context_files` works regardless and is what Layer 1 falls back to until [T20260510-12] lands.

This is the universal layer because every supported agent runtime (Claude, Codex, Gemini, Anthropic API, OpenAI-compat, Ollama, mock) consumes a prompt. The injection is invisible to the runtime.

**Limitation.** This layer fires once per task, before the agent has read its way into the relevant files. Learnings whose scope is narrower than the task's overall scope may not surface here; that's what layers 2 and 3 are for.

### 4.2 Layer 2 — MCP tool-call injection (cross-agent, fine-grained)

For MCP tools whose arguments or responses reference file paths — `orbit_task_show` (which surfaces `context_files`), `orbit_task_artifact_put`, and similar registered tools — the `orbit-remote` MCP composition attaches a `learnings` sidecar to the tool response. The CLI-only graph surface does not pass through this mechanism:

```jsonc
{
  "result": { ... },
  "learnings": [
    {
      "id": "L-0001",
      "summary": "Never declare a perf win on latency alone — ..."
    }
  ]
}
```

The agent's MCP client surfaces the sidecar however it normally surfaces tool output. Modern agents read structured tool responses; the sidecar is part of that response, so it lands in agent context naturally.

This layer covers any agent that talks to Orbit's Remote-composed MCP server. It does not cover agent-vendor-specific tools (e.g. Claude Code's built-in `Edit`/`Write`/`Read`), which the MCP server doesn't see. Layer 3 fills that gap for Claude Code specifically.

### 4.3 Layer 3 — Claude Code `PreToolUse` hook (optional)

A `PreToolUse` hook in [.claude/settings.json](../../../.claude/settings.json) intercepts `Edit | Write | Read`, extracts the target path from the tool input, calls `orbit learning list --path <path>` (post-[ORB-00202] glob-containment semantics), and emits a `<system-reminder>` with the matching learnings before the tool runs.

This is the only layer that surfaces learnings on Claude Code's built-in editor tools, which agents use far more than they call MCP file tools. It's the most precise layer (per-edit, per-target) but the least universal (Claude Code only).

The hook is shipped as part of the design, but it is **layered on top of** layers 1 and 2, not a replacement. Other agent vendors that gain analogous hook capabilities can plug in equivalent layers without touching the Orbit-side store.

Every hook fire that admits at least one learning records a `learning_injected` audit event in the host-global `~/.orbit/orbit.db` (`arguments_json.learning_ids`, target path, session id). The instrumentation **fails open**: an unavailable audit backend logs a warning and the reminder still renders — injection never depends on the observability write ([ORB-10316]). The per-learning rollup over these events is described in [§5.5](#55-usage-instrumentation-and-feedback).

The hook resolves its session identity in priority order: `ORBIT_SESSION_ID` from the environment (engine-managed runs export it, pre-seeded with layer-1 injections) → the `session_id` field carried by the hook payload itself (Claude Code sends it on every hook event) → a tmpdir file keyed by parent pid as the last resort. The payload fallback matters: interactive sessions never export `ORBIT_SESSION_ID`, and each hook fire runs under a fresh shell, so the ppid fallback re-keys per invocation and cannot dedup (the 2026-07-18 relevancy audit observed one learning injected 10× in a single session; F2026-07-092).

### 4.4 Deduplication and budget

A naive implementation injects the same learning multiple times across layers (e.g. once at layer 1, once at layer 2 for a tool call referencing the same file, once at layer 3 for the eventual edit). To prevent this:

- The agent process tracks injected learning IDs in a per-session set.
- Each layer consults the set before emitting a `<system-reminder>`; already-injected IDs are skipped.
- Per-call cap of **5** learnings (configurable via `ORBIT_LEARNING_PER_CALL_CAP`). Hard cap of **20** per session (configurable via `ORBIT_LEARNING_SESSION_CAP`) to bound total context cost.

Implementation note: the per-session set lives in the agent's working memory. The Orbit-side store does not need to track session state; it just provides idempotent search. Layers consult the set; the store is stateless.

Cross-process deduplication is best-effort and keyed on a session id: `ORBIT_SESSION_ID` when exported, else (for Layer 3) the `session_id` field the hook payload carries ([ORB-10316]). With a session id, dedup state lives in the `session_learning_state` table of the host-global `orbit.db`; without one, Layer 3 falls back to a tmpdir state file keyed by parent pid, which cannot dedup across fires from an interactive agent (each fire gets a fresh parent shell). In-process Layer 1 + Layer 2 dedup is exact; an `orbit-mcp` server started outside an engine-spawned session still falls back to per-process state and may double-emit. The dedup layer is belt-and-braces; the agent's own context window remains the practical backstop.

### 4.5 What gets injected

The injected teaser carries only the learning **id**, one-line **summary**, and scope **tags** (`- [L-0001] summary [tags: a, b]`). `body` is **not** injected — bodies are loaded on demand via `orbit learning show` / `orbit.learning.show`. This keeps per-injection token cost small (a few dozen tokens per learning, not a few hundred), which is what makes the 5-per-call cap workable.

If an agent decides a teaser is relevant, it pulls the body explicitly — and that `show` is the passive usage signal the deprecation rollup reads (see [§5.5](#55-usage-instrumentation-and-feedback)). This separates "alerting the agent that a learning exists" from "spending context on the full content." Most learnings will be teaser-only in any given session.

---

## 5. CLI and MCP Surface

### 5.1 CLI

```
orbit learning add --summary <text> --scope paths=... [tags=...] [--body-file FILE] [--evidence task=T... commit=SHA ...]
orbit learning list [--status active|superseded] [--tag TAG] [--path GLOB]  # --path uses glob-containment
orbit learning show <id>                  # loads the full body; emits a learning_shown usage signal
orbit learning update <id> [--summary ...] [--body-file ...] [--scope ...]
orbit learning supersede <id> --with <new-id>
orbit learning upvote --id <id> --model <agent-family> --task <task-id>
orbit learning sync                       # reconcile SQLite index from YAML
orbit learning prune [--stale-only]       # report or delete stale learnings
orbit learning stats [--since 30d]        # per-learning injected/shown usage rollup (see §5.5)

# Free-text content match (formerly the per-domain `learning` subcommand of `orbit search`) lives on the unified search surface:
orbit search <text> --kind learning [--tag T] [--all] [--status learning:active] [--limit N]
orbit search path <path> --kind learning [--tag T] [--all] [--status learning:active]
```

`add`, `update`, and `supersede` write the YAML and update the index atomically. `upvote` appends to the learning's `votes.jsonl` sidecar and is idempotent for `(learning_id, voter_model, task_id)`. `orbit learning list --path/--tag` and `orbit search --kind learning` are the fast read paths used by the injection layers (the runtime-side `search_learnings` helper is the in-process equivalent).

### 5.2 MCP tools

| Tool | Inputs | Outputs |
|------|--------|---------|
| `orbit.learning.add` | `summary`, `scope`, `body?`, `evidence?` | `{ id, created_at }` |
| `orbit.learning.list` | `status?`, `tag?`, `path?` (glob-containment) | `{ learnings: [...] }` |
| `orbit.search` (`kind: "learning"`) | `query?`, `tag?`, `path?`, `limit?`, `all?`, `status?` | ranked list with `kind: "learning"` hits |
| `orbit.learning.show` | `id` | full record plus vote summary |
| `orbit.learning.update` | `id`, fields | updated record |
| `orbit.learning.supersede` | `id`, `with` | both records updated |
| `orbit.learning.upvote` | `id`, `model`, `task?` | vote summary |

`orbit.learning.list` and the runtime-side `search_learnings` helper drive the injection-layer hot path; both must stay sub-10ms at expected scale. The standalone per-domain learning-search MCP tool (phase-1 surface) was retired by [ORB-00202] in favor of `orbit.search` with `kind: "learning"`.

### 5.3 Result shape

```jsonc
{
  "results": [
    {
      "id": "L-0001",
      "summary": "Never declare a perf win on latency alone — ...",
      "tags": ["performance", "benchmarking"],
      "matched_by": ["path:crates/orbit-knowledge/src/graph_bench.rs", "tag:performance"],
      "updated_at": "2026-05-09T18:00:00Z"
    }
  ]
}
```

`matched_by` is exposed deliberately: agents can see which scope axis triggered the match, which feeds back into both human curation (is the path glob right?) and future ranking work.

### 5.4 Re-validation votes

When an agent finds an existing learning that covers a duplicate concern, it records a re-validation signal instead of authoring a competing record:

```jsonc
{
  "learning_id": "L-0001",
  "voter_model": "claude",
  "voted_at": "2026-05-17T12:00:00Z",
  "task_id": "ORB-00095"
}
```

Rows append to `.orbit/learnings/<id>/votes.jsonl` using `O_APPEND`; each learning has its own file and lock, so cross-learning contention is zero. V1 rejects free-floating votes without `task_id` to keep the signal anchored to a concrete work context. Duplicate rows with the same `(learning_id, voter_model, task_id)` are treated as one vote, preserving the earliest timestamp for that key.

`orbit.learning.show` reports derived vote fields: `vote_count` and `last_voted_at`. `orbit.learning.list` and `orbit.search` (with `kind: "learning"`) keep their envelope output shape unchanged.

Search ranking remains scope-filtered first. Within the matched set, rows sort by:

1. decay-weighted vote score, default half-life 180 days;
2. manual `priority`;
3. `updated_at` desc;
4. `id` asc.

`ORBIT_LEARNING_VOTE_HALF_LIFE_DAYS=0` disables decay and uses raw vote count. Vote files are scanned at query time in v1; a SQLite vote-summary mirror is a follow-up only if measured matched-set sizes make the per-file scan visible.

### 5.5 Usage instrumentation and feedback

Two audit event kinds in the host-global `~/.orbit/orbit.db` carry the usage signal ([ORB-10316], [ADR-0242]; the scoped reintroduction of a feedback primitive anticipated by [ADR-0210]):

- **`learning_injected`** — one event per hook fire that admitted learnings; `arguments_json.learning_ids` lists the injected IDs, `target_id` is the tool-target path, `session_id` the resolved session.
- **`learning_shown`** — recorded when an agent opens a learning's full body via `orbit learning show` (CLI) or the `orbit.learning.show` MCP tool: `target_id` is the learning ID, keyed by session. This is the **passive, ungameable usage signal** — an agent that expands a teaser found it worth reading. There is deliberately **no ack surface** (no `orbit learning ack`, no `orbit.learning.ack`): an earlier ack-based design ([ADR-0242] rejected alternatives) added a gameable, agent-remembered step and a new MCP tool; `show` needs neither.

Both emissions **fail open**: an unavailable audit backend logs a warning and the injection/show still completes. The signal is best-effort observability and must never break the read or injection path.

`orbit learning stats [--since ..] [--json]` folds both event kinds into a per-learning rollup: injected count, shown count, shown ratio (`shown / injected`), and last-injected/last-shown timestamps (CLI + the `learning_usage_stats` runtime API). A low shown ratio — injected often, never read — is the deprecation-candidate signal. This rollup is the designed input for downstream deprecation policy (ORB-10318); decay/TTL is deliberately follow-up work, not implemented at this layer.

---

## 6. Pull Surface

### 6.1 `orbit-learnings` skill

A skill at `.claude/skills/orbit-learnings/` (and the equivalent location for other agent vendors) exists for the active-query path. Trigger phrases include "what should I know about", "are there learnings for", "is there context I'm missing on". The skill body documents how to call `orbit.search` (with `kind: "learning"`) and how to interpret results.

The skill is the pull complement to push. Push handles the "agent doesn't know it should look" failure mode; the skill handles the "agent has time to ask" case (e.g., at task start, when reviewing an unfamiliar area).

### 6.2 Direct tool use

Agents that don't load skills can call `orbit.search` (with `kind: "learning"`) directly via MCP. The tool's input schema is documented; its output shape matches §5.3.

### 6.3 Dashboard

The local dashboard exposes learnings under Knowledge > Learnings. The HTTP surface is deliberately thin over the same runtime helpers used by CLI/MCP:

- `GET /api/learnings` lists records with optional `q`, `scope`, `tag`, `limit`, and `offset` filters and returns dashboard stats (`total`, `superseded`, `last_indexed`).
- `GET /api/learnings/:id` returns the full record.
- `POST /api/learnings/:id/supersede` accepts `{ "by": "<replacement-learning-id>" }` and runs the same atomic supersession path described in §7.2.

The dashboard is a pull and curation surface, not an injection layer. It lets operators scan stale or duplicate records before review without changing the phase-1 push semantics.

---

## 7. Curation Lifecycle

### 7.1 Authoring

Learnings are authored by:

- Agents at the end of a task — when an agent recognizes "this is the kind of correction that will keep happening." The `orbit-learnings` skill covers the `orbit.learning.add` flow.
- Humans during code review or after incidents — same surface, manual invocation.

The bar for authoring: the knowledge must be **non-obvious** (otherwise it lives in code), **not-feature-scoped** (otherwise it's an ADR), and **load-bearing across more than one task** (otherwise it's a comment in a single PR).

### 7.2 Supersession

When a learning is replaced by a clearer or more current entry:

```
orbit learning supersede L-0001 --with L-0042
```

Both records update atomically. The old record's `status` flips to `superseded` and gains a `superseded_by` field; the new record's `supersedes` field points back. Superseded records are excluded from injection but retained on disk for history.

### 7.3 Staleness detection

A learning is **stale** if any of these are true:

- All files matching `scope.paths` no longer exist.
- All `evidence` commit SHAs no longer exist on the active branch.
- All `evidence` task IDs are deleted.

`orbit learning prune --stale-only` reports staleness; with `--delete` it archives the record. Staleness detection is opportunistic, not automatic; nothing fires it on every commit. Phase 2 may wire it into the knowledge graph rebuild path.

### 7.4 Conflict resolution

Two agents (or two humans) may author overlapping learnings concurrently. Phase 1 does not auto-merge; the curation answer is "humans review and supersede one with the other when the duplication surfaces." `orbit learning list --tag <tag>` is the manual surface for spotting duplicates. Phase 2's semantic-similarity ranking will naturally surface near-duplicates at injection time, which is the better forcing function.

### 7.5 Re-validation without re-authoring

When a duplicate concern is already covered by an active learning, the agent should upvote the existing record instead of creating a near-duplicate. The vote says "this learning is still load-bearing in a new task context" and improves search ranking without changing the learning body or `updated_at`.

### 7.6 Recurring deprecation review (auto-task)

`orbit learning prune` ([§7.3](#73-staleness-detection)) is mechanical and anchor-only: it flags a learning stale purely on dead paths / evidence, and it is opportunistic — nothing schedules it. The teaser-injection instrumentation ([ORB-10316], [§5.5](#55-usage-instrumentation-and-feedback)) adds a second, softer signal — *whether a learning is ever injected or shown* — that `prune` does not consider. A learning can be perfectly anchored (its globs still resolve) yet be dead weight: never injected, injected-but-never-opened, or scoped so broadly it only ever matches tangentially.

To surface that class continuously without hard-coding thresholds, orbit ships a **report-only recurring review** as an auto-task ([docs/design/auto-tasks/](../auto-tasks/), [ORB-10318]) — `.orbit/auto_tasks/learning-deprecation-review.yaml`. On its cadence the generic scheduler mints a normal task whose prompt directs the assigned agent to:

1. Read the usage rollups (`orbit learning stats --json`: `injected_count`, `shown_count`, `shown_ratio`, `last_injected_at`, `last_shown_at`) and each learning's age (`orbit learning list --json`).
2. Inspect **anchor health** from `scope.paths`: empty path scopes (can never inject) and globs that match nothing in the current tree.
3. Write a ranked list of the potentially stale learnings, each with evidence, into the task's `execution_summary` — that is the entire deliverable.

The run **never** deprecates, deletes, supersedes, archives, or adds a learning state; curation stays human/orchestrator-owned and is applied afterwards through the existing `orbit learning update` / archival surface. It **fails open**: an empty or missing rollup (a fresh workspace with no audit history) reports "nothing stale" rather than erroring. Agent judgment replaces the fixed thresholds a bespoke staleness-scoring engine would have hard-coded — the deliberate choice ([ORB-10318]) not to build one, given the small corpus and the audit's finding that no learning is a proven chronic offender.

The definition is ordinary workspace data (`no-diff-expected` + `learning-deprecation` tags, `skip_if_open` dedupe); its cadence lives in the definition's `schedule` field, not in the identity `config.yaml` ([L-0014] keeps runtime config out of `config.yaml`). First-cycle calibration reproduces the 2026-07-18 hook-relevancy audit's candidate set (L-0074, L-0077, L-0068, L-0041, and the six empty-path learnings) or documents why any now differs.

---

## 8. Concerns & Honest Limitations

### 8.1 Authoring discipline is the bottleneck

The system can be perfect at *delivering* learnings and still fail if no one *writes* them. The `orbit-learnings` skill and the agent-self-authoring flow are the primary remediations, but neither is automatic. If authoring lags, the store stays sparse and the push layer surfaces nothing — same end state as today, just with more code in the way.

This is acknowledged, not fixed. Phase 2's auto-extraction from review threads or postmortems may help; phase 1 ships with manual authoring and accepts the discipline cost.

### 8.2 Path globs are brittle to large refactors

A learning scoped to `crates/orbit-knowledge/src/graph_bench.rs` becomes invisible the day someone moves the file. Tags partly compensate (tag-based scoping survives renames) but require the author to anticipate the rename, which is rare.

Phase 2's symbol-aware scope handles renames cleanly because the knowledge graph tracks symbol identity across moves. Phase 1's mitigation is operational: when a refactor moves files, run `orbit learning prune --stale-only` and update or supersede affected records as part of the refactor task.

### 8.3 Vote ranking still depends on agent discipline

Phase 1 ranks matched learnings by decayed upvotes before falling back to manual priority and recency. This is better than recency-only ranking, but it depends on agents recording votes only when they have genuinely evaluated a duplicate concern. Over-eager upvoting would make the signal noisy. The v1 mitigations are task-anchored idempotency and time decay, not a full abuse-prevention system.

Phase 2's semantic-similarity ranking from orbit-search may complement or replace parts of this formula; vote score is a load-bearing signal, not the whole relevance model.

### 8.4 Layer 3 hook is Claude-Code-only

The `PreToolUse` hook covers Claude Code's built-in `Edit | Write | Read`, which are the most frequent agent actions. Other agents that gain comparable hooks can layer in equivalent integrations, but as of phase 1, agents without hook support get only layers 1 and 2 — coarser-grained injection. This is uneven coverage by agent vendor; the design accepts the unevenness because layer 1 is universal and gives a baseline that's strictly better than today.

### 8.5 Per-session deduplication depends on a resolvable session id

Cross-process dedup state is session-keyed in `orbit-store` (`session_learning_state`), but it only engages when a session id resolves — from `ORBIT_SESSION_ID` or, for Layer 3, the hook payload ([ORB-10316]). Agents whose hook payloads carry no session id and that run outside engine management still fall back to the per-invocation ppid state file and may re-inject. Separately, the agent's own in-context dedup set resets on context compression or crash-restart; the store-side state is the backstop for exactly the sessions that can be keyed.

### 8.6 No write-time validation that learnings are non-obvious

Authoring policy ([§7.1](#71-authoring)) is enforced by reviewer judgment, not by the tool. Nothing prevents an agent from writing a "learning" that just restates what `Cargo.toml` says. Quality control is a curation problem, not a schema problem; phase 1 ships without programmatic guardrails and relies on the same review pressure that keeps `MEMORY.md` and ADR logs honest.

### 8.7 Privacy posture

Learnings are workspace-scoped and checked into the repo. They travel exactly where the repo travels. There is no telemetry surface in the loop and no remote API. The authoritative content (the YAML) stays local by construction, like task content. The only shared artifact is the rebuildable SQLite envelope index in the host-global `orbit.db`, and it is partitioned by `workspace_id` so one workspace never reads another's rows (ADR-0212).

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [T20260510-12] — Add `tags` field to `Task` schema. Hard prerequisite for Layer 1's tag-axis matching ([§4.1](#41-layer-1--engine-pre-prompt-injection-universal)).
- [ORB-00061] — Add Knowledge tab and Learnings subtab to dashboard.
- [ORB-00090] — Aligned learning identity examples with the agent-family convention.
- [ORB-10316] — Teaser injection (id + summary + tags), `learning_shown` usage signal on `orbit learning show`, `learning_injected`/`learning_shown` rollup via `orbit learning stats`, payload-derived session dedup ([§4.3](#43-layer-3--claude-code-pretooluse-hook-optional), [§4.5](#45-what-gets-injected), [§5.5](#55-usage-instrumentation-and-feedback); [ADR-0242]).
- [ORB-10318] — Report-only recurring learning-deprecation review as an auto-task; surfaces stale candidates via `execution_summary` from usage rollups + anchor health, no bespoke sweep engine ([§7.6](#76-recurring-deprecation-review-auto-task)).

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
