---
name: orbit-task
description: The task lifecycle — create a task, execute one through implementation and handoff, review someone else's work, and file friction when Orbit tooling or skill guidance is itself the problem. Not for vague task content (re-author it) or ordinary bugs (those are tasks).
---

# Orbit Task

Every `orbit.task.*` call needs `model` (your agent family); see the `orbit` skill for the MCP/CLI surface mapping. Never use bare `orbit task ...` — it skips agent provenance.

Two modes live in their own references, loaded when you need them:

- Reviewing someone else's work → [references/review.md](references/review.md)
- Filing tooling, workflow, or skill-guidance friction → [references/friction.md](references/friction.md)

## 1. Create

Create a task another engineer or agent can execute without guessing: a crisp problem description plus strong acceptance criteria. The execution plan is authored later, at pickup, and persisted with `orbit.task.update`.

1. Confirm objective, constraints, and done criteria.
2. Check for overlapping prior work with `orbit-search` (`hybrid: true`, `kind: "task"`).
3. Write acceptance criteria that define observable success — name a command, inspection step, or observable output for each. "Works correctly" is not a criterion.
4. Optionally fill `context_files` (see below).
5. Set `complexity` (`low`/`medium`/`hard`) whenever scope is clear enough to judge — dispatch batching honors it.
6. Add assumptions, risks, and rollback notes to the description when they matter.
7. Call `orbit.task.add`. Confirm via the result or re-fetch with `orbit.task.show`.

**`context_files`** names *only* modification and deletion targets, as canonical selectors (`file:`, `dir:`, `symbol:path#name:kind`), each resolving inside the target workspace's root — an out-of-root path fails pipeline admission. Read-for-context files, convention and pattern docs, and files that don't exist yet do not belong there; cite those in prose instead. The exception is a design doc your repo co-locates with the code it describes, since it co-changes. Prefer `file:`/`symbol:` over `dir:` when the change can be named precisely. The field is optional unless your workspace's own policy requires it: leaving it empty is valid, and guessing entries to avoid an empty field is worse than empty.

**Operating rules.** Never edit task files directly; never invent task IDs (`orbit.task.add` allocates them). Required: `title`, `description`, `workspace` — strongly prefer `acceptance_criteria` and `complexity`. `description` should be multi-line markdown for anything non-trivial. Valid `type`: `feature`, `bug`, `refactor`, `chore`. Do not pass the retired `plan` field. Blank companion files (`plan.md`, `execution-summary.md`) are blank *fields* — repair with `orbit.task.update`, never by hand.

**Behavior-affecting optional fields:**

- `dependencies: ["ORB-NNNN", ...]` — prerequisites must reach a satisfying status first.
- `relations: [{"type": "resolves", "target": "F<YYYY>-<MM>-<NNN>"}]` — auto-resolves that friction when this task reaches `done`. Other types (`produces`, `blocked_by`, `child_of`, `spawned_from`, `regression_from`, `supersedes`, `related_to`) are tracked but inert. Only `produces`/`resolves` accept non-`ORB-` targets; the rest require `ORB-NNNNN`. A dangling target succeeds but emits a `TaskRelationDangling` audit event.
- `parent_id`, `source_task_id` (the bug-introducing task; creation-time only — `update` silently drops it), `tags` (reuse existing before inventing new).

**Quality bar.** Validation must not assume uncommitted artifacts or workspace-local runtime state under `.orbit/state/`. File I/O checks use temp dirs or fakes. Behavior-changing work that touches external services, the filesystem, or time should ask for deterministic mock coverage in its acceptance criteria.

Description template:

```markdown
## Problem
<what is broken, missing, or needs to change>
## Why It Matters
<user impact, operational impact, or engineering rationale>
## Constraints / Notes
- <important constraint>
```

Exit: the task exists with a strong description, clear acceptance criteria, and — when filled — `context_files` naming only real modification targets.

### Preparing selectors under traffic

An empty `context_files` list is valid while a task is being authored. When an
orchestrator needs selectors prepared before reservation or conflict checks,
use the existing task-pilot job rather than filling the list inline:

```bash
orbit job show task_pilot_pipeline
orbit run job task_pilot_pipeline
```

The zero-input mode discovers only proposed/backlog tasks with empty
`context_files`; pass explicit `task_ids` to audit exactly named tasks. Its
apply step persists only validated selectors, reducing file-collision risk. An
enabled workspace routine may already run the zero-input job, but an extra run
is appropriate when ship traffic is high.

## 2. Execute

Carry a task from intent to verified implementation with explicit lifecycle tracking.

**Step 1 — Load.** `orbit.task.show`, then extract `description`/`acceptance_criteria` (the required outcome), `plan` (author one if blank or placeholder), `context_files`, and `status`. Read each `file:` target with the provider-native file-read tool. For a `dir:` selector, do not call the file-read tool on the directory: after verifying it resolves beneath the workspace root, use `rg --files <directory>` to list it, then use the provider-native file-read tool only for its key files. Then `orbit.search` with `semantic: "<task-id>"`, `limit: 5` to surface prior related decisions — non-blocking, skip it if nothing is relevant. No ID yet? Clarify intent with the human, then use Create above.

**Step 2 — Plan.** `orbit.task.update` with a concrete markdown `plan` — target files, validation commands, risks — if one doesn't exist.

**Step 3 — Start.** `orbit.task.start` with a `note`. Moves `backlog`/`proposed` → `in-progress` and records approval. Starting from `proposed` still requires a real plan.

**Step 4 — Implement and validate.** Follow the plan, inspecting files with the provider-native file-read tool. Verify transitive impact with `rg` or by reading callers directly. Run the repo-approved verification commands, honoring repo instructions if tests are forbidden.

If implementation surfaces a file outside the original `context_files`, append its canonical selector via `orbit.task.update` as soon as you discover it — don't wait until handoff. Reservation itself is the system's job, not yours: there is no worker-callable lock tool, and none should be reached for. Keeping `context_files` current is how a worker excludes others from files it owns, since conflict detection reads live from in-flight tasks, not only from reservation records. One caveat: an added entry only binds reservations requested *after* the update — it does not retroactively revoke a reservation a concurrent run already holds.

In a linked pipeline worktree, never use positional `git stash` / `git stash pop`: refs and the stash list are repository-global, so a positional pop can restore another session's work. Record the worktree's initial `git rev-parse HEAD` and compare against that explicit baseline with `git diff <baseline-sha> -- <paths>`.

**Step 5 — Summarize and hand off.** Persist `execution_summary` via `orbit.task.update` first. Then consider friction: if the task surfaced a contradicted assumption, a recurring failure mode, a non-obvious gotcha, or an incident root cause, file it (see [references/friction.md](references/friction.md)). Then:

- **Under an activity envelope** (e.g. `agent_implement`): persist the summary only. The pipeline owns the `review` transition after commit/merge/PR steps succeed.
- **Direct execution** (no envelope): persist the summary *and* move to `review` via `orbit.task.update`.

The generated PR body already supplies `## Task` / `## Execution Summary` / `## Validation` / `## Branch Freshness`, so don't duplicate those headings. Required content:

```markdown
Outcome: success | failed
Changes:
- <what changed and why>
Assessment: <short quality assessment>
```

Include when relevant: `Strategic decisions:`, `Design weaknesses / risks:` (with Severity/Mitigation), `Deviations from original plan:` (with Justification), `Recommended follow-ups:`.

**Lifecycle rules.** One task per activity invocation — no multiplexing. Ask clarifying questions before implementing if material ambiguity remains. If approval for `proposed` work can't be obtained, stop after recording that state. Direct execution must persist a non-empty `execution_summary` before or with the review transition.

Exit: task started via `orbit.task.start`; execution summary persisted; friction checkpoint considered; direct execution advanced to `review`.
