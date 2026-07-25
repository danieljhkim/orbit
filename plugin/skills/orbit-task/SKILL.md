---
name: orbit-task
description: The task-lifecycle skill — create a task, execute an existing Orbit task or human request through the lifecycle with explicit status tracking, review someone else's work and surface findings in a review summary, and file self-reported friction when Orbit tooling or skill instructions cause operational problems. Triggers on "create a task", "review T-id", "review this PR", "leave review feedback", tool failures, wrong CLI behavior, or misleading skill guidance. Not for task content issues like vague descriptions (fix those by re-authoring) or ordinary user-requested work/generic bugs (use friction only for self-reported Orbit tooling/workflow friction).
---

# Orbit Task

Both surfaces (MCP `orbit_task_*` / CLI `orbit tool run orbit.task.*`) accept identical JSON; **always include `model`** (your agent family: `codex`, `claude`, `gemini`, or `grok` — full strings auto-normalize). See the `orbit` skill for the full surface-mapping rule. Never use direct `orbit task ...` CLI — it skips agent provenance; use `orbit tool run orbit.task.<action>` instead.

## 1. Create

Create a task another engineer or agent can execute without guessing: a crisp problem description plus strong acceptance criteria. The execution plan is authored later, at pickup.

**Workflow:**
1. Confirm objective, constraints, and done criteria.
2. Optionally check for overlapping prior work with `orbit-search` (`hybrid: true`, `kind: "task"`).
3. Write acceptance criteria that define observable success — no vague pass/fail language like "works correctly"; name a command, inspection step, or observable output for each.
4. Enumerate files/dirs/symbols this task will modify or delete as canonical selectors (`file:`, `dir:`, `symbol:path#name:kind`) in `context_files`. Only modification targets — not read-for-context files, conventions/pattern docs (cite those in prose instead), or files that don't exist yet. Design docs your repo co-locates with the code they describe are the exception (co-change with implementation). Prefer `file:`/`symbol:` over `dir:` when changes can be named more precisely.
5. Set `complexity` (`low`/`medium`/`hard`) whenever scope is clear enough to judge — batching honors this when dispatching work.
6. Add assumptions, risks, and rollback notes to the description when they matter.
7. Call `orbit.task.add` with description, acceptance criteria, `context_files`, workspace, complexity, and `model`. Leave `plan` blank unless pre-seeding is justified.
8. Confirm via the tool result, or re-fetch with `orbit.task.show`.

**Operating rules:** Never edit task files directly. Never invent task IDs — `orbit.task.add` allocates them. `description` should be multi-line markdown for non-trivial tasks. Required: `title`, `description`, `workspace`. Strongly prefer `acceptance_criteria` and `complexity`. Valid `type`: `feature`, `bug`, `refactor`, `chore` — use friction (below) for self-reported tooling issues, not a task type. Blank/missing companion files (`plan.md`, `execution-summary.md`) are blank fields — repair via `orbit.task.update`, never by hand.

**Behavior-affecting optional fields:**
- `dependencies: ["ORB-NNNN", ...]` — prerequisite tasks must reach a satisfying status first.
- `relations: [{"type": "resolves", "target": "F<YYYY>-<MM>-<NNN>"}]` — auto-resolves the target friction when this task reaches `done`. Other `relations` types (`produces`, `blocked_by`, `child_of`, `spawned_from`, `regression_from`, `supersedes`, `related_to`) are tracked; only `produces`/`resolves` accept non-`ORB-` targets (friction/learning/ADR IDs), the rest require `ORB-NNNNN`. Dangling `resolves`/`produces` targets succeed but emit a `TaskRelationDangling` audit event.
- `parent_id`, `source_task_id` (bug-introducing task; creation-time only — `update` silently drops it on existing tasks), `tags` (reuse existing before inventing new).

**Task quality bar:** validation must not assume `.orbit/knowledge/` or uncommitted artifacts; file I/O checks use temp dirs/fakes; behavior-changing tasks touching external services/FS/time should call for deterministic mock coverage in acceptance criteria; graph/knowledge task nodes define `purpose` as role + crate/module + leaf-or-internal.

```bash
orbit tool run orbit.task.add --input '{
  "title": "<title>", "description": "<multi-line markdown>",
  "acceptance_criteria": ["<observable outcome 1>", "<observable outcome 2>"],
  "context_files": ["file:src/lib.rs", "dir:src/command", "symbol:src/lib.rs#run:function"],
  "plan": "", "workspace": "<path>", "priority": "<low|medium|high|critical>",
  "complexity": "<low|medium|hard>", "type": "<feature|bug|refactor|chore>",
  "model": "<agent-family>"
}'
```

Description template:

```markdown
## Problem
<what is broken, missing, or needs to change>
## Why It Matters
<user impact, operational impact, or engineering rationale>
## Constraints / Notes
- <important constraint>
```

Exit: task exists with strong description, clear acceptance criteria, and (when applicable) `context_files` naming only real modification targets via canonical selectors.

## 2. Execute

Carry a task (or human request, once created above) from intent to verified implementation with explicit lifecycle tracking.

**Step 1 — Load or create.** Given an existing ID, `orbit.task.show` and extract `description`/`acceptance_criteria` (required outcome), `plan` (author one if blank/placeholder), `context_files` (with shell access, resolve via `orbit graph show`/`orbit graph overview`; otherwise use `fs.read`), `status`. Then call `orbit.search` with `semantic: "<task-id>"`, `limit: 5` (non-blocking — skip if the companion is missing or nothing's relevant) to surface prior related decisions. No ID yet → clarify intent with the human, then use Create above.

**Step 2 — Plan.** `orbit.task.update` with a concrete markdown `plan` (target files, validation commands, risks) if one doesn't already exist.

**Step 3 — Start.** `orbit.task.start` with a `note`. Moves `backlog`/`proposed` → `in-progress` (records approval automatically). Starting from `proposed` still requires a real plan.

**Step 4 — Implement and validate.** Follow the plan; use the CLI-only `orbit graph` surface when shell access is available, otherwise inspect files directly. Verify transitive impact during implementation/review with `orbit graph` or `rg`; run the repo-approved verification commands (honor repo instructions if tests are forbidden).

**Step 5 — Summarize and hand off.** Persist `execution_summary` via `orbit.task.update` first (template below). Friction checkpoint: if the task surfaced a contradicted assumption, recurring failure mode, non-obvious gotcha, or incident root cause, file it with `orbit.friction.add` (see §4). Project learnings are curated by the workspace's orchestrator or owner, not by task executors — the learning authoring gate refuses `orbit.learning.add`/`update`/`supersede` from executor context, so calling `orbit-knowledge`'s `add` action here is a wasted call, not a judgment call. Skip filing if none of the trigger conditions apply. Then:
- **Under an activity envelope** (e.g. `agent_implement`): persist the summary only — the pipeline owns the `review` transition after commit/merge/PR steps succeed.
- **Direct execution** (no envelope): persist the summary and move to `review` via `orbit.task.update`.

Execution summary — required content (the generated PR body supplies `## Task`/`## Execution Summary`/`## Validation`/`## Branch Freshness`; don't duplicate those headings):

```markdown
Outcome: success | failed
Changes:
- <what changed and why>
Assessment: <short quality assessment>
```

Include when relevant: `Strategic decisions:`, `Design weaknesses / risks:` (with Severity/Mitigation), `Deviations from original plan:` (with Justification), `Recommended follow-ups:`.

**Lifecycle rules:** one task per activity invocation — no multiplexing. Ask clarifying questions before implementing if material ambiguity remains. If `proposed`-work approval can't be obtained, stop after recording that state. Don't skip lifecycle updates. Direct execution must persist a non-empty `execution_summary` before/with the review transition.

Exit: task started via `orbit.task.start`; execution summary persisted; friction checkpoint considered; direct execution advanced to `review` (envelope-driven execution leaves that to the pipeline).

## 3. Review

Review someone else's work and surface issues in your review summary — read-only; **never** transition the reviewed task's lifecycle.

**Load context.** `orbit.task.show` for `description`/`acceptance_criteria`/`plan`/`execution_summary`; inspect the diff and changed files; run the target repo's build and the relevant test commands (from its own instructions/configuration). Optionally `orbit.search` with `semantic: "<task-id>"` for prior similar decisions.

**Two-stage review — stage 1 first:** spec compliance (does the change satisfy every acceptance criterion? anything missing or added beyond scope? interpretation gaps?). If it fails, report those findings and stop — don't spend time on stage 2. **Stage 2** (only if stage 1 passes): maintainability, patterns, performance, test-coverage gaps, risks/edge cases/security.

**Record findings** one per distinct issue — cite `path:line` when location-specific, general otherwise:

```text
**[Spec compliance | Code quality | Nit] — short headline.**
Why this matters / what's wrong.
Suggested fix.
```

**Summarize** in chat: finding count, which are blockers, overall verdict (approve/request changes).

**Meta-review.** After recording findings, check whether they reveal a gap in an Orbit-authored instruction asset (an activity definition or a `SKILL.md`). Trigger: ≥2 findings map to the same instruction gap, or one finding is clearly a recurring class. When it fires, file a friction (see §4) *in addition to* the individual findings — never as a replacement. Skip for a single nit, a style preference, or a one-off mistake with no link to instruction text.

**Rules:** never transition the reviewed task's status; skip stylistic nits when a blocking issue already stops the change.

**Not for:** implementing a task (§2); a task lifecycle approval (`orbit.task.approve`, owned by the reviewee/human).

Exit: all findings recorded with a clear verdict; no status transitions on the reviewed task; chat summary names blocker count and verdict.

## 4. Friction (self-reported tooling/skill issues)

File **agent-discovered Orbit tooling, workflow, or seeded-instruction friction** as an append-only report instead of silently working around it — unclear command behavior, missing CLI functionality, confusing schema/config, doc gaps, unclear errors, unexpected runtime behavior, confusing seed instructions, or insufficient/vague activity-asset or `SKILL.md` prompts. **Not** for task content issues, ordinary user-requested work, or generic bugs.

Friction bodies are append-only markdown reports under `.orbit/frictions/`; their triage metadata is mutable. New records start `open` and may be `triaged` or `resolved`. Human/orchestrator triage uses `orbit friction update <ID> --status triaged` (and optional replacement tags), while `orbit friction resolve <ID>` closes a report. When a task fixes the underlying cause, add `relations: [{"type":"resolves","target":"F<YYYY>-<MM>-<NNN>"}]`: that task auto-resolves an existing friction and records `resolved_by_task` when it reaches `done`; a dangling target is audit-visible but does not block task completion.

| Tag | Use for |
| --- | --- |
| `build` | make/fmt/lint friction |
| `docs` | Stale/missing CLAUDE.md or design docs |
| `lifecycle` | Task lifecycle confusion/transition issues |
| `naming` | Naming drift or duplicated sources of truth |
| `policy` | fsProfile or sandboxing surprises |
| `skill-guidance` | Misleading or incorrect skill instructions |
| `tooling` | Orbit tool/CLI/MCP failures |
| `other` | Fallback |

```bash
orbit tool run orbit.friction.add --input '{
  "body": "<what happened, where, and why it caused friction>",
  "tags": ["<tag from table>"], "during_task": "<optional task id>",
  "model": "<agent-family>"
}'
```

**Rules:** never silently ignore an Orbit problem — always report; never implement large design changes inline, track them first; name the concrete command/file/workflow that broke; report genuine friction only.
