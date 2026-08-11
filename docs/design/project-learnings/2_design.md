---
summary: "Project Learnings — Design"
type: design
title: "Project Learnings — Design"
owner: claude
last_updated: 2026-08-10
status: Draft
feature: project-learnings
doc_role: design
tags: ["project-learnings"]
---

# Project Learnings — Design

This document specifies phase-1 project-learnings: the placement of learning storage in `orbit-store`, the schema of a learning record plus sidecars, the phase-1 scope-matching algorithm (path globs + tags), pull-based discovery through search and show, the reference-comment convention, the curation lifecycle, and the concerns the design deliberately leaves to follow-ups.

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

`orbit.search` and `orbit.learning.show` are the delivery surface. Search returns candidate records without expanding their bodies; `show` returns the authoritative body and records a passive `learning_shown` usage event. Code and workflow boundaries can point to a relevant record with a concise reference comment, keeping the rationale close to use without copying durable content into the source.

No cross-crate dependencies that violate the architecture diagram in [CLAUDE.md](../../../CLAUDE.md) are introduced. The dependency edges remain `orbit-store` (extended internally) and the existing consumers of the learning tools; delivery does not add an agent-runtime or hook-path dependency.

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
    - "benchmarks/identity-key/**"
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

  **Why:** A prior performance comparison showed a speedup that turned out to
  come from the new path silently dropping output.

  **How to apply:** When working on `benchmarks/identity-key/**` or any
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

The legacy flat layout (`.orbit/learnings/<id>.yaml` plus `.orbit/learnings/superseded/<id>.yaml`) is rejected on load with an actionable migration error. `orbit learning migrate-layout` reports the one-way move without changing files; `orbit learning migrate-layout --confirm` performs it and leaves `tags.yaml` at `.orbit/learnings/tags.yaml` ([ORB-10452]).

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

`L-NNNN` — allocated per workspace by `orbit.learning.add` on the owning machine, never invented by agents (same rule as task IDs). Unlike task IDs, which carry a machine-scoped prefix chosen at global init, learning IDs have no machine component: uniqueness is `(workspace_id, id)` and two workspaces may both hold an `L-0007`.

---

## 3. Scope Matching (phase 1)

Phase 1 supports two scope axes, evaluated as a logical OR:

### 3.1 Path globs

Glob patterns over repo-relative paths. They narrow explicit `orbit search path <path> --kind learning` and `orbit learning list --path <path>` queries.

Glob syntax: standard `**`/`*`/`?` semantics (the same matcher `orbit-policy` uses for `read`/`modify` rules — reused, not reimplemented). A learning matches if **any** of its `scope.paths` matches the candidate path.

### 3.2 Tags

Free-form string labels. They narrow explicit `orbit search --kind learning --tag <tag>` and `orbit learning list --tag <tag>` queries.

Tags are not auto-derived from anything in phase 1. They exist for the cases where path-based scoping doesn't fit ("when running any benchmark", "when authoring docs").

### 3.3 Combination

A learning matches an explicit query if **(path glob matches) OR (any tag matches)**. The OR is deliberate: the two axes capture different shapes of relevance and shouldn't gate each other.

### 3.4 Why not symbol-aware in phase 1

Symbol-aware scoping (e.g. "this learning applies whenever the agent touches
the `cosine_similarity` function regardless of where it lives") is more
precise than path globs, but Orbit has no live symbol resolver. The schema
continues to preserve the unused `scope.symbols` field for compatibility; any
future implementation requires a fresh design ([3_vision.md §1.1](./3_vision.md)).

---

## 4. Pull Delivery and Reference Comments

### 4.1 Discover and retrieve

Agents start with `orbit search --kind learning <query>` or a path/tag-constrained query to discover candidate records. They retrieve a full record only with `orbit learning show <id>` or `orbit.learning.show`. This keeps a search result concise while preserving a single authoritative body for the durable guidance.

### 4.2 Point-of-use references

When a learning, ADR, task, or friction report constrains a specific code or workflow boundary, the surrounding source may carry a one-line reference comment: `// L-0041: hook subcommands keep parsing and state in core.` Reference comments must include the artifact ID and a short claim about why it applies. They are intentionally small pointers, not duplicated policy text.

Do not place workspace-local artifact IDs in shipped skills, prompt templates, or other consumer-facing instruction surfaces: those IDs are not portable outside this workspace. The artifact registry and the reference comment together are the delivery mechanism.

### 4.3 Historical injection data

Only the Claude Code `PreToolUse` hook registration was removed from the repository settings ([ORB-10346], 2026-07-20); it was the sole layer that emitted the `learning_injected` audit event (`crates/orbit-cmd/src/learning_hook.rs`). Engine pre-prompt injection (`maybe_prepend_learning_reminders` in `crates/orbit-engine/src/activity_job/agent_loop_driver.rs`) and the MCP sidecar decorator (`LearningSidecarDecorator` in `crates/orbit-remote/src/mcp/learning.rs`, registered in `crates/orbit-remote/src/mcp/mod.rs`) remain active and fire on every run, but neither emits `learning_injected` — so `learning_injected` audit events and their counters are frozen as of 2026-07-20 and read as historical calibration data, not as evidence that push delivery stopped. `learning_shown` continues to be emitted when an agent explicitly opens a learning and is the active usage signal.

---

## 5. CLI and MCP Surface

### 5.1 CLI

```
orbit learning add --summary <text> --scope paths=... [tags=...] [--body-file FILE] [--evidence task=T... commit=SHA ...]
orbit learning list [--status active|superseded] [--tag TAG] [--path GLOB]  # --path uses glob-containment
orbit learning show <id>                  # loads the full body; emits a learning_shown usage signal
orbit learning update <id> [--summary ...] [--body-file ...] [--scope ...]
orbit learning supersede <id> --with <new-id>
orbit learning archive <id>               # retire a single learning without a replacement [ORB-10469]
orbit learning upvote --id <id> --model <agent-family> --task <task-id>
orbit learning sync                       # reconcile SQLite index from YAML
orbit learning prune [--stale-only]       # report or delete stale learnings
orbit learning stats [--since 30d]        # per-learning injected/shown usage rollup (see §5.5)

# Free-text content match (formerly the per-domain `learning` subcommand of `orbit search`) lives on the unified search surface:
orbit search <text> --kind learning [--tag T] [--all] [--status learning:active] [--limit N]
orbit search path <path> --kind learning [--tag T] [--all] [--status learning:active]
```

`add`, `update`, and `supersede` write the YAML and update the index atomically. `upvote` appends to the learning's `votes.jsonl` sidecar and is idempotent for `(learning_id, voter_model, task_id)`. `orbit learning list --path/--tag` and `orbit search --kind learning` are the indexed read paths used for pull discovery.

**Authoring is role-gated ([ADR-0250], [ORB-10364]; extended to `archive` by [ORB-10469]).** `add`, `update`, `supersede`, and `archive` — and only those four — refuse callers running in an agent-executor context, returning a `policy denied` error that names `orbit friction add` as the correct channel and echoes the attempted content so the observation is not lost. The role comes from the `ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL` identity pair the audit middleware already reads: present ⇒ agent, absent ⇒ human. An orchestrator that dispatches curation work *as* an agent opts in deliberately with `ORBIT_LEARNING_AUTHOR=1`. Every read surface, plus `sync`, `prune`, and `stats`, is unaffected in every context.

### 5.2 MCP tools

| Tool | Inputs | Outputs |
|------|--------|---------|
| `orbit.learning.add` | `summary`, `scope`, `body?`, `evidence?` | `{ id, created_at }` |
| `orbit.learning.list` | `status?`, `tag?`, `path?` (glob-containment) | `{ learnings: [...] }` |
| `orbit.search` (`kind: "learning"`) | `query?`, `tag?`, `path?`, `limit?`, `all?`, `status?` | ranked list with `kind: "learning"` hits |
| `orbit.learning.show` | `id` | full record plus vote summary |
| `orbit.learning.update` | `id`, fields | updated record |
| `orbit.learning.supersede` | `id`, `with` | both records updated |
| `orbit.learning.archive` | `id` | retired record; idempotent on an already-superseded `id` ([ORB-10469]) |
| `orbit.learning.upvote` | `id`, `model`, `task?` | vote summary |

`orbit.learning.list` and `orbit.search` are the primary discovery paths; both must stay sub-10ms at expected scale. The standalone per-domain learning-search MCP tool (phase-1 surface) was retired by [ORB-00202] in favor of `orbit.search` with `kind: "learning"`.

`orbit.learning.add`, `orbit.learning.update`, `orbit.learning.supersede`, and `orbit.learning.archive` carry the same [ADR-0250] caller-role gate as their CLI counterparts — the check lives on the shared `OrbitRuntime::author_learning*` surface, so the two entry points cannot drift. A refused tool call returns `{"code": "policy_denied", "error": ...}` and writes nothing.

### 5.3 Result shape

```jsonc
{
  "results": [
    {
      "id": "L-0001",
      "summary": "Never declare a perf win on latency alone — ...",
      "tags": ["performance", "benchmarking"],
      "matched_by": ["tag:performance"],
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

Two audit event kinds in the host-global `~/.orbit/orbit.db` describe historical delivery and current explicit use:

- **`learning_injected`** — historical evidence from the retired Claude Code hook layer only. Its final values are frozen as of 2026-07-20; the system must tolerate no new events. Engine pre-prompt injection and the MCP sidecar decorator continue to inject on every run without emitting this event — the freeze reflects that audit source going away, not push delivery stopping (see [§4.3](#43-historical-injection-data)).
- **`learning_shown`** — recorded when an agent opens a learning's full body via `orbit learning show` (CLI) or the `orbit.learning.show` MCP tool. This is the active, passive usage signal for explicit retrieval.

`learning_shown` emission **fails open**: an unavailable audit backend logs a warning and the `show` read still completes. The signal is best-effort observability and must never break retrieval.

`orbit learning stats [--since ..] [--json]` retains its per-learning rollup: historical injected count, shown count, shown ratio (`shown / injected`), and last-injected/last-shown timestamps (CLI + the `learning_usage_stats` runtime API). It handles a zero-new-injections future without error. The deprecation review treats the frozen injection figures as calibration data and relies on current shown data and anchor health for ongoing assessment ([ORB-10318]).

---

## 6. Pull Surface

### 6.1 `orbit-learnings` skill

A skill at `.claude/skills/orbit-learnings/` (and the equivalent location for other agent vendors) exists for the active-query path. Trigger phrases include "what should I know about", "are there learnings for", "is there context I'm missing on". The skill body documents how to call `orbit.search` (with `kind: "learning"`) and how to interpret results.

The skill is the primary delivery path for agents that need guidance at task start or while reviewing an unfamiliar area. A nearby reference comment supplies the same pointer at a specific code or workflow boundary.

### 6.2 Direct tool use

Agents that don't load skills can call `orbit.search` (with `kind: "learning"`) directly via MCP. The tool's input schema is documented; its output shape matches §5.3.

### 6.3 Dashboard

The local dashboard exposes learnings under Knowledge > Learnings. The HTTP surface is deliberately thin over the same runtime helpers used by CLI/MCP:

- `GET /api/learnings` lists records with optional `q`, `scope`, `tag`, `limit`, and `offset` filters and returns dashboard stats (`total`, `superseded`, `last_indexed`).
- `GET /api/learnings/:id` returns the full record.
- `POST /api/learnings/:id/supersede` accepts `{ "by": "<replacement-learning-id>" }` and runs the same atomic supersession path described in §7.2.

The dashboard is a pull and curation surface. It lets operators scan stale or duplicate records before review without changing the delivery model.

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
orbit learning supersede L-0001 --with L-0002
```

Both records update atomically. The old record's `status` flips to `superseded` and gains a `superseded_by` field; the new record's `supersedes` field points back. Superseded records are excluded from default discovery but retained on disk for history.

### 7.2.1 Archival (retirement without a replacement) [ORB-10469]

Supersession requires a replacement ID; a verified-obsolete learning with no successor (e.g. its subject feature was deleted wholesale, F2026-07-100) previously had no sanctioned single-record retirement path — only the indiscriminate `prune --delete` sweep reached the underlying `archive_learning` store write, and it archives every stale record, not one named ID.

```
orbit learning archive <id>
```

Flips `status` to `superseded` with `superseded_by: null` — the same terminal state `prune --delete` writes, reached by name instead of by staleness heuristic. Archiving an already-superseded record is an idempotent no-op: it leaves `superseded_by` exactly as it was (whether that is `null` from a prior archive, or a real replacement ID from a prior `supersede`) rather than clobbering it. `archive` carries the same [ADR-0250] caller-role gate as `add` / `update` / `supersede`, via `OrbitRuntime::author_learning_archive`.

### 7.3 Staleness detection

A learning is **stale** if any of these are true:

- All files matching `scope.paths` no longer exist.
- All `evidence` commit SHAs no longer exist on the active branch.
- All `evidence` task IDs are deleted.

`orbit learning prune` reports staleness; with `--confirm` it archives the
record. The former `--delete` spelling remains a compatibility alias.
Staleness detection is opportunistic, not automatic; nothing fires it on every
commit.

### 7.3.1 Approved physical retirement

Physical retirement is distinct from staleness pruning. Use it only for a specifically approved learning ID or for records whose persisted lifecycle status is `superseded`; never infer retirement from age, path drift, or zero usage. In the current schema the only retired status is `superseded` (there is no `deprecated` variant); legacy data, if introduced, must be enumerated explicitly before any deletion.

The sanctioned sequence is:

1. Enumerate the approved IDs and lifecycle-status candidates, then grep the workspace for each ID. Rewrite every surviving reference comment so the code states the rationale without pointing to the retiring learning.
2. Remove the complete `.orbit/learnings/<id>/` artifact directory. Do not edit `learning.yaml` in place.
3. Run `orbit learning sync --json` to rebuild the workspace's `learnings_index` projection from the remaining artifacts. This removes the deleted record's index row; no checked-in manifest points to learning artifacts. The workspace's own sequence keeps its high-water mark so a deleted ID is never reused.
4. Re-run the ID grep and confirm the directory and index-backed listing no longer surface the record.

This is distinct from `orbit learning archive` ([§7.2.1](#721-archival-retirement-without-a-replacement-orb-10469)): archival retains a superseded artifact, while approved physical retirement removes the artifact after its references have been made self-contained.

### 7.4 Conflict resolution

Two agents (or two humans) may author overlapping learnings concurrently. Phase 1 does not auto-merge; the curation answer is "humans review and supersede one with the other when the duplication surfaces." `orbit learning list --tag <tag>` is the manual surface for spotting duplicates. Phase 2's semantic-similarity ranking can surface near-duplicates during search.

### 7.5 Re-validation without re-authoring

When a duplicate concern is already covered by an active learning, the agent should upvote the existing record instead of creating a near-duplicate. The vote says "this learning is still load-bearing in a new task context" and improves search ranking without changing the learning body or `updated_at`.

### 7.6 Recurring deprecation review (auto-task)

`orbit learning prune` ([§7.3](#73-staleness-detection)) is mechanical and anchor-only: it flags a learning stale purely on dead paths / evidence, and it is opportunistic — nothing schedules it. The historical injection counts and current `learning_shown` signal ([§5.5](#55-usage-instrumentation-and-feedback)) add contextual evidence that `prune` does not consider. A learning can be perfectly anchored yet be dead weight: never shown, or associated with a reference comment that no longer explains a live boundary. Point-of-use reference comments ([§4.2](#42-point-of-use-references)) can go stale the same way — the artifact they cite may since have been deleted, rejected, or superseded.

To surface both classes continuously without hard-coding thresholds, orbit ships a **report-only recurring review** as an auto-task ([docs/design/auto-tasks/](../auto-tasks/), [ORB-10318], generalized by [ORB-10348]) — `.orbit/auto_tasks/artifact-deprecation-review.yaml`. On its cadence the generic scheduler mints a normal task whose prompt directs the assigned agent to gather evidence from two streams:

**Stream A — learning corpus health** (original scope):

1. Read the usage rollups (`orbit learning stats --json`: `injected_count`, `shown_count`, `shown_ratio`, `last_injected_at`, `last_shown_at`) and each learning's age (`orbit learning list --json`).
2. Inspect **anchor health** from `scope.paths`: empty path scopes and globs that match nothing in the current tree.

**Stream B — comment-reference sweep** (added by [ORB-10348]):

1. Grep the reachable constellation checkout for artifact-id patterns in comments — `L-\d{4}`, `ADR-\d{4}`, `[A-Z]{2,5}-\d{5}` (task IDs now carry a machine-scoped prefix, so `ORB-` is no longer the only one), `F\d{4}-\d{2}-\d{3}`.
2. Resolve each id against its source (`orbit learning show`, `orbit task show`, `orbit friction show`; for ADRs, the `## ADR-NNNN` heading in the owning feature's `4_decisions.md`) and report references whose artifact is missing, rejected, superseded, or otherwise stale, with `file:line` evidence and the comment's claim versus the artifact's current state. ADR anchors get this check for free once the §11 lint in [../CONVENTIONS.md](../CONVENTIONS.md#11-enforcement) exists.

Both streams write into the task's `execution_summary` — a ranked list of candidates with concrete evidence — and that is the entire deliverable. The run **never** deprecates, deletes, supersedes, archives, or adds state to any learning, ADR, task, or friction record, and never edits a comment; curation stays human/orchestrator-owned and is applied afterwards through the existing `orbit learning update` / archival surface, or by hand for comment edits. It **fails open** per stream: an empty or missing rollup, a sweep with no matches, or otherwise missing/empty data reports "nothing stale" for that stream rather than erroring — a fresh workspace with no audit history, or a codebase with no reference comments yet, is a valid "nothing to deprecate" outcome, not a failure. Agent judgment replaces the fixed thresholds a bespoke staleness-scoring engine would have hard-coded — the deliberate choice ([ORB-10318]) not to build one, given the small corpus and the audit's finding that no learning is a proven chronic offender.

The definition is ordinary workspace data (`no-diff-expected` + `artifact-deprecation` tags, `skip_if_open` dedupe); its cadence lives in the definition's `schedule` field, not in the identity `config.yaml` ([L-0014] keeps runtime config out of `config.yaml`). The 2026-07-18 hook-relevancy audit's injection figures are frozen historical calibration as of 2026-07-20; the review documents why any candidate differs from that baseline.

---

## 8. Concerns & Honest Limitations

### 8.1 Authoring discipline is the bottleneck

The system can be perfect at storing learnings and still fail if no one writes or discovers them. The `orbit-learnings` skill, reference comments, and the agent-self-authoring flow are the primary remediations, but none is automatic. If authoring lags, the store stays sparse.

This is acknowledged, not fixed. Phase 2's auto-extraction from review threads or postmortems may help; phase 1 ships with manual authoring and accepts the discipline cost.

### 8.2 Path globs are brittle to large refactors

A learning scoped to a benchmark source file becomes invisible the day someone moves the file. Tags partly compensate (tag-based scoping survives renames) but require the author to anticipate the rename, which is rare.

The mitigation is operational: when a refactor moves files, run
`orbit learning prune --stale-only` and update or supersede affected records as
part of the refactor task.

### 8.3 Vote ranking still depends on agent discipline

Phase 1 ranks matched learnings by decayed upvotes before falling back to manual priority and recency. This is better than recency-only ranking, but it depends on agents recording votes only when they have genuinely evaluated a duplicate concern. Over-eager upvoting would make the signal noisy. The v1 mitigations are task-anchored idempotency and time decay, not a full abuse-prevention system.

Phase 2's semantic-similarity ranking from orbit-search may complement or replace parts of this formula; vote score is a load-bearing signal, not the whole relevance model.

### 8.4 Pull delivery requires a useful locator

Search depends on a meaningful query, and a reference comment can drift after a refactor. Keep comments concise, cite the artifact ID, and use the recurring review to surface stale anchors. This cost is accepted in exchange for avoiding automatic context delivery on unrelated tool calls.

### 8.5 No write-time validation that learnings are non-obvious

Authoring policy ([§7.1](#71-authoring)) is enforced by reviewer judgment, not by the tool. Nothing prevents an agent from writing a "learning" that just restates what `Cargo.toml` says. Quality control is a curation problem, not a schema problem; phase 1 ships without programmatic guardrails and relies on the same review pressure that keeps `MEMORY.md` and ADR logs honest.

### 8.6 Privacy posture

Learnings are workspace-scoped and checked into the repo. They travel exactly where the repo travels. There is no telemetry surface in the loop and no remote API. The authoritative content (the YAML) stays local by construction, like task content. The only shared artifact is the rebuildable SQLite envelope index in the host-global `orbit.db`, and it is partitioned by `workspace_id` so one workspace never reads another's rows (ADR-0212).

---

## Task References

- [T20260510-11] — Design + build project-learnings system as native Orbit primitive. The task that produced this folder.
- [T20260510-12] — Add `tags` field to `Task` schema.
- [ORB-00061] — Add Knowledge tab and Learnings subtab to dashboard.
- [ORB-00090] — Aligned learning identity examples with the agent-family convention.
- [ORB-10316] — Added `learning_shown` and the `learning_injected`/`learning_shown` stats rollup retained as historical data ([§5.5](#55-usage-instrumentation-and-feedback)).
- [ORB-10318] — Report-only recurring learning-deprecation review as an auto-task; surfaces stale candidates via `execution_summary` from usage rollups + anchor health, no bespoke sweep engine ([§7.6](#76-recurring-deprecation-review-auto-task)).
- [ORB-10346] — Removed the Claude Code `PreToolUse` hook layer of automatic learning delivery and added pull delivery with reference comments; engine pre-prompt injection and the MCP sidecar decorator remain active ([§4.3](#43-historical-injection-data)).
- [ORB-10348] — Generalized the review into `artifact-deprecation-review`: added the comment-reference sweep (Stream B) alongside the original learning-corpus-health stream ([§7.6](#76-recurring-deprecation-review-auto-task)).
- [ORB-10364] — Gated the `add`/`update`/`supersede` authoring surfaces on caller role and redirected executors to `friction add` ([§5.1](#51-cli), [§5.2](#52-mcp-tools), ADR-0250).
- [ORB-10452] — Made learning layout migration and stale-learning pruning non-destructive by default, with the shared non-interactive `--confirm` apply convention.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
