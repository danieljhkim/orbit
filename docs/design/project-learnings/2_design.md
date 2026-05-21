---
summary: "Project Learnings — Design"
type: design
title: "Project Learnings — Design"
owner: claude
last_updated: 2026-05-21
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

`orbit-mcp` gains a thin shim that, for tool responses referencing file paths, attaches a `learnings` sidecar field with up to N matching entries. This is the second push layer; it works for any agent that calls Orbit's MCP tools.

The third push layer — a Claude Code `PreToolUse` hook on `Edit | Write | Read` — is not part of any Orbit crate; it ships as a hook configuration in [.claude/settings.json](../../../.claude/settings.json) (or whichever scope is appropriate; see [§4.3](#43-layer-3-claude-code-pretooluse-hook-optional)).

No cross-crate dependencies that violate the architecture diagram in [CLAUDE.md](../../../CLAUDE.md) are introduced. The dependency edges added are `orbit-store` (extended internally), `orbit-tools → orbit-store` (already present), and `orbit-engine → orbit-store` (already present). `orbit-mcp` remains a transport adapter that depends only on `orbit-common`; Layer 2 asks the injected host to run `orbit.search` (with `kind: "learning"`) instead of reading the learning store directly.

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

A SQLite table `learnings_index` mirrors a few columns for fast scope matching, since brute-forcing path globs over every YAML on every tool call is the wrong shape:

```sql
CREATE TABLE learnings_index (
    id          TEXT PRIMARY KEY,         -- L-0001
    status      TEXT NOT NULL,            -- "active" | "superseded"
    paths       TEXT NOT NULL,            -- JSON array of glob patterns
    tags        TEXT NOT NULL,            -- JSON array of tags
    summary     TEXT NOT NULL,            -- denormalized for fast read
    updated_at  TEXT NOT NULL
);

CREATE INDEX learnings_active ON learnings_index(status) WHERE status = 'active';
```

Query path: filter to `status = 'active'`, load the small set of `(paths, tags)` rows, run the in-memory glob match. At expected scale (low hundreds of active learnings), this is sub-millisecond; the index exists to avoid YAML I/O on every tool call.

The YAML files are the source of truth. The index is rebuildable from them via `orbit learning reindex`.

Vote rows are source-of-truth sidecars, not SQLite projections in v1. `orbit learning reindex` still walks every per-learning `votes.jsonl` and fails on invalid JSONL, so cache rebuilds do not silently ignore corrupted vote files.

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

For tools whose arguments or responses reference file paths — `orbit_graph_show`, `orbit_graph_refs`, `orbit_task_show` (which surfaces `context_files`), `orbit_task_artifact_put`, etc. — `orbit-mcp` attaches a `learnings` sidecar to the tool response:

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

This layer covers any agent that talks to Orbit's MCP server. It does not cover agent-vendor-specific tools (e.g. Claude Code's built-in `Edit`/`Write`/`Read`), which `orbit-mcp` doesn't see. Layer 3 fills that gap for Claude Code specifically.

### 4.3 Layer 3 — Claude Code `PreToolUse` hook (optional)

A `PreToolUse` hook in [.claude/settings.json](../../../.claude/settings.json) intercepts `Edit | Write | Read`, extracts the target path from the tool input, calls `orbit learning list --path <path>` (post-[ORB-00202] glob-containment semantics), and emits a `<system-reminder>` with the matching learnings before the tool runs.

This is the only layer that surfaces learnings on Claude Code's built-in editor tools, which agents use far more than they call MCP file tools. It's the most precise layer (per-edit, per-target) but the least universal (Claude Code only).

The hook is shipped as part of the design, but it is **layered on top of** layers 1 and 2, not a replacement. Other agent vendors that gain analogous hook capabilities can plug in equivalent layers without touching the Orbit-side store.

### 4.4 Deduplication and budget

A naive implementation injects the same learning multiple times across layers (e.g. once at layer 1, once at layer 2 for a tool call referencing the same file, once at layer 3 for the eventual edit). To prevent this:

- The agent process tracks injected learning IDs in a per-session set.
- Each layer consults the set before emitting a `<system-reminder>`; already-injected IDs are skipped.
- Per-call cap of **5** learnings (configurable via `ORBIT_LEARNING_PER_CALL_CAP`). Hard cap of **20** per session (configurable via `ORBIT_LEARNING_SESSION_CAP`) to bound total context cost.

Implementation note: the per-session set lives in the agent's working memory. The Orbit-side store does not need to track session state; it just provides idempotent search. Layers consult the set; the store is stateless.

Cross-process deduplication is best-effort via `ORBIT_SESSION_ID` plus `.orbit/state/sessions/<id>/learnings.json`. In-process Layer 1 + Layer 2 dedup is exact; when Layer 2 or Layer 3 runs without `ORBIT_SESSION_ID` (for example, an `orbit-mcp` server started outside an engine-spawned session, or a Claude session not initiated through Orbit), they fall back to per-process state and may double-emit. The dedup layer is belt-and-braces; the agent's own context window remains the practical backstop.

### 4.5 What gets injected

`summary` is always injected. `body` is **not** — bodies are loaded on demand via `orbit.learning.show`. This keeps per-injection token cost small (a few dozen tokens per learning, not a few hundred), which is what makes the 5-per-call cap workable.

If an agent decides a summary is relevant, it pulls the body explicitly. This separates "alerting the agent that a learning exists" from "spending context on the full content." Most learnings will be summary-only in any given session.

---

## 5. CLI and MCP Surface

### 5.1 CLI

```
orbit learning add --summary <text> --scope paths=... [tags=...] [--body-file FILE] [--evidence task=T... commit=SHA ...]
orbit learning list [--status active|superseded] [--tag TAG] [--path GLOB]  # --path uses glob-containment
orbit learning show <id>
orbit learning update <id> [--summary ...] [--body-file ...] [--scope ...]
orbit learning supersede <id> --with <new-id>
orbit learning upvote --id <id> --model <agent-family> --task <task-id>
orbit learning reindex                    # rebuild SQLite index from YAML
orbit learning prune [--stale-only]       # report or delete stale learnings

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

### 8.5 Per-session deduplication state is agent-local

The dedup set lives in the agent's working memory. If an agent's context is compressed or the agent crashes and restarts, the set resets and the same learning may inject twice. Hardening this would require a session-keyed cache in `orbit-store`, which trades complexity for a marginal context-token saving. Not worth it in phase 1.

### 8.6 No write-time validation that learnings are non-obvious

Authoring policy ([§7.1](#71-authoring)) is enforced by reviewer judgment, not by the tool. Nothing prevents an agent from writing a "learning" that just restates what `Cargo.toml` says. Quality control is a curation problem, not a schema problem; phase 1 ships without programmatic guardrails and relies on the same review pressure that keeps `MEMORY.md` and ADR logs honest.

### 8.7 Privacy posture

Learnings are workspace-scoped and checked into the repo. They travel exactly where the repo travels. There is no telemetry surface in the loop, no remote API, no shared store across workspaces. Like task content, learning content stays local by construction.

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [T20260510-12] — Add `tags` field to `Task` schema. Hard prerequisite for Layer 1's tag-axis matching ([§4.1](#41-layer-1--engine-pre-prompt-injection-universal)).
- [ORB-00061] — Add Knowledge tab and Learnings subtab to dashboard.
- [ORB-00090] — Aligned learning identity examples with the agent-family convention.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
