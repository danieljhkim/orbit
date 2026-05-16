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
5. Call the task-add tool (`orbit_task_add` over MCP, or `orbit tool run orbit.task.add` from the shell) with the description, acceptance criteria, workspace, and exact `model` field in the JSON input. Orbit infers the agent family from known model names. Leave `plan` blank unless you have a compelling reason to pre-seed it.
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

### Tier 2 - Mention
- `parent_id: "ORB-NNNN"` metadata: only for real subtask-of relationships; display/list grouping and batch relatedness.
- `source_task_id: "ORB-NNNN"` metadata: for `type: bug`, names the task that introduced the defect; display-only today.

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
  "model": "<model_name>" # gpt-5.4, claude-opus-4-6, gemini-2.5-pro, etc
  # Optional: complexity, dependencies, parent_id, source_task_id, tags - see "Optional but Behavior-Affecting Fields"
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
  "model": "<model_name>"
  # Optional: complexity, dependencies, parent_id, source_task_id, tags - see "Optional but Behavior-Affecting Fields"
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
