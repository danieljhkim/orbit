---
name: orbit-create-task
description: Use this when you need to create an Orbit task.
---

# Orbit Create Task

## Purpose

Create an Orbit task another engineer or agent can execute without guessing. Focus task creation on a crisp description of the problem and strong acceptance criteria; the execution plan is authored later when the task is picked up.

## Tool Invocation

Two surfaces, identical JSON args:

- **MCP**: `orbit_task_add({...})` when the orbit plugin is connected.
- **CLI**: `orbit tool run orbit.task.add --input '{...}'` from the shell.

See the `orbit` skill for the full mapping rule and surface coverage. Examples below use CLI form; substitute `orbit_task_add` with the same JSON when MCP tools are loaded.

## Workflow

1. Confirm objective, constraints, and done criteria.
2. Inspect codebase context before creating the task. If you want background on prior related work, `orbit.semantic.search` is available (hybrid BM25 + cosine over indexed task fields) — useful when the proposed work might overlap with a task whose title uses different vocabulary. Optional, not required. See `orbit-semantic`.
3. Write clear acceptance criteria that define observable success.
4. Add assumptions, risks, and rollback notes to the description when they matter.
5. Call the task-add tool (`orbit_task_add` over MCP, or `orbit tool run orbit.task.add` from the shell) with the description, acceptance criteria, workspace, and canonical `model` family in the JSON input. Use `codex`, `claude`, `gemini`, or `grok`; full model strings are accepted and auto-normalized. Leave `plan` blank unless you have a compelling reason to pre-seed it.
6. Use the result as the default confirmation. If you need to re-fetch the canonical stored record, call `orbit_task_show({"id": "<returned-id>"})` (MCP) or `orbit tool run orbit.task.show --input '{"id": "<returned-id>"}'` (CLI).

## Selector-First Context

- Prefer canonical task context selectors in `context_files`: `file:path`, `dir:path`, and `symbol:path#name:kind`.
- Raw legacy paths are still accepted, but Orbit silently upgrades them to canonical selector form on write.
- Add `context_files` entries only for existing files, directories, or symbols expected to be modified or deleted by the task.
- Do not add entries solely for files that will be created later, or for files that are only relevant background context.
- Prefer `file:` selectors over `dir:` selectors whenever the expected changes can be named at file level; use `dir:` only when the directory itself is the smallest honest scope.
- When a task needs precise code context, prefer `symbol:` selectors over whole-file scopes.

## Operating Rules

- Never edit task files directly.
- Never invent task IDs.
- `description` should be multi-line markdown when the task is non-trivial.
- Required fields: `title`, `description`, and `workspace`.
- Strongly prefer supplying `acceptance_criteria`.
- Blank or missing task companion files (`plan.md`, `execution-summary.md`) are treated as blank task fields. Repair them through `orbit.task.update` (`plan` or `execution_summary`), not manual file edits.
- Orbit fills `created_by`, `planned_by`, and `implemented_by` automatically from execution context when those roles are authored during the task lifecycle.
- Valid task types are `feature`, `bug`, `refactor`, and `chore`. Use `orbit-track-issues` for agent self-reported friction instead of task types.

## Optional but Behavior-Affecting Fields

### Tier 1 - Nudge
- `complexity: "hard"` trigger: set when the task obviously cannot share a batch (large surface, multi-crate cross-cut, ambiguous design).
  Behavior anchor: `crates/orbit-engine/src/executor/automation/batch/dispatch.rs` `task_prefers_single_batch`.
- `dependencies: ["ORB-NNNN", ...]` trigger: set when prerequisite tasks must reach a dependency-satisfying status before this task starts.
  Behavior anchor: `crates/orbit-common/src/types/task.rs` `task_dependencies_ready`.
- `relations: [{"type": "resolves", "target": "F<YYYY>-<MM>-<NNN>"}]` trigger: set when this task closes a tracked friction. On the Review → Done approval transition, the targeted friction is auto-resolved (`status: resolved`, `resolved_at: now`, `resolved_by_task: <this-task-id>`). Drop the structured relation at task-creation time so closure flows from the lifecycle, not a manual `orbit.friction.resolve` follow-up.
  Behavior anchor: `crates/orbit-core/src/command/task/transitions.rs` `apply_resolves_side_effects`.

### Tier 2 - Mention
- `parent_id: "ORB-NNNN"` metadata: only for real subtask-of relationships; display/list grouping and batch relatedness.
- `source_task_id: "ORB-NNNN"` metadata: for `type: bug`, names the task that introduced the defect. Settable at task-creation only — `orbit.task.update` currently silently drops this field on existing tasks (see friction `F2026-05-024` / task `ORB-00101`).

### Cross-Artifact Relations

The full `relations` array accepts these typed variants. Only the first two accept non-`ORB-` targets:

- `produces` — this task created the target artifact during execution. Targets: `ORB-NNNNN`, `F<YYYY>-<MM>-<NNN>` (friction), `L<YYYYMMDD>-N` (learning), `ADR-NNNN`. Tracking-only in v1 (no lifecycle side-effect).
- `resolves` — this task closes or supersedes the target artifact. Same target set as `produces`. **Side-effect when target is a friction**: auto-resolve on Review → Done (see Tier 1 above). Other target kinds are tracked but not state-mutated in v1.
- `blocked_by`, `child_of`, `spawned_from`, `regression_from`, `supersedes`, `related_to` — task-only. Target must be `ORB-NNNNN`; cross-artifact targets are rejected by validation.

Dangling targets (e.g., `resolves` pointing at a non-existent friction) succeed at approval time but emit a `TaskRelationDangling` audit event — they do not roll the task back.

### Tier 3 - Tags
Tags are indexed by `orbit.semantic.search`; use existing tags where they fit before inventing new ones, because speculative tag soup is costly.

## Task Quality Standards

### Validation Environment checklist

- Validation must not assume the presence of `.orbit/knowledge/`, local config files, or filesystem artifacts that are neither committed nor created by the task itself.
- When automated tests are in scope, file I/O checks must use temp directories or in-memory fakes.
- Acceptance criteria should state filesystem expectations explicitly when behavior touches persisted state.

### Explicit Definitions checklist

- Terms such as `purpose`, `summary`, and `description` in acceptance criteria must include an observable format or output requirement.
- If acceptance criteria mention a `signature`, specify the exact form it must take.
- Prohibit vague pass/fail language such as `works correctly` or `handles edge cases`; replace it with concrete observable behavior.

### Mock-Based, Deterministic Testing

- When repo policy allows automated coverage, tasks that modify behavior involving external services, filesystem I/O, or time-dependent state should call for deterministic mock or fake coverage in acceptance criteria.
- Prefer implementation patterns that remain compatible with mocks or fakes instead of hard-coding runtime type checks that block test doubles.
- Acceptance criteria should specify expected return types or output shapes when functions change.

### Per-Node Purpose checklist

- For graph or knowledge tasks, define each node `purpose` as: role in one sentence, crate or module, and whether the node is leaf or internal.

### AC Format Rule

- Each acceptance criterion must be independently verifiable and name a command, inspection step, or observable output.
- When a safe validation command exists, include at least one acceptance criterion that names it explicitly.

## Command

CLI form:

```bash
orbit tool run orbit.task.add --input '{
  "title": "<title>",
  "description": "<multi-line markdown>",
  "acceptance_criteria": [
    "<observable outcome 1>",
    "<observable outcome 2>"
  ],
  "plan": "",
  "context_files": ["file:src/lib.rs", "dir:src/command", "symbol:src/lib.rs#run:function"],
  "workspace": "<absolute_or_relative_repo_path>",
  "priority": "<low|medium|high|critical>",
  "type": "<feature|bug|refactor|chore>",
  "model": "<agent-family>" # codex | claude | gemini | grok
  # Optional: complexity, dependencies, relations, parent_id, source_task_id, tags - see "Optional but Behavior-Affecting Fields"
}'
```

MCP form (same JSON, called as `orbit_task_add`):

```text
orbit_task_add({
  "title": "<title>",
  "description": "<multi-line markdown>",
  "acceptance_criteria": ["<observable outcome 1>", "<observable outcome 2>"],
  "context_files": ["file:src/lib.rs", "symbol:src/lib.rs#run:function"],
  "workspace": "<absolute_or_relative_repo_path>",
  "priority": "<low|medium|high|critical>",
  "type": "<feature|bug|refactor|chore>",
  "model": "<agent-family>"
  # Optional: complexity, dependencies, relations, parent_id, source_task_id, tags - see "Optional but Behavior-Affecting Fields"
})
```

## Description Template

```markdown
## Problem
<what is broken, missing, or needs to change>

## Why It Matters
<user impact, operational impact, or engineering rationale>

## Constraints / Notes
- <important constraint>
- <relevant context>
```

## Exit Criteria

The task exists with a strong description, clear acceptance criteria, and enough context for a later planning phase to succeed.
