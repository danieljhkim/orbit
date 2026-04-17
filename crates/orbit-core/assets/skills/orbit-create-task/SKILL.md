---
name: orbit-create-task
description: Use this when you need to create an Orbit task.
---

# Orbit Create Task

## Purpose

Create an Orbit task another engineer or agent can execute without guessing. Focus task creation on a crisp description of the problem and strong acceptance criteria; the execution plan is authored later when the task is picked up.

## Workflow

1. Confirm objective, constraints, and done criteria.
2. Inspect codebase context before creating the task.
3. Write clear acceptance criteria that define observable success.
4. Add assumptions, risks, and rollback notes to the description when they matter.
5. Run `orbit tool run orbit.task.add` with the description, acceptance criteria, and workspace. Leave `plan` blank unless you have a compelling reason to pre-seed it.
6. Use the `orbit.task.add` result as the default confirmation. If you need to confirm the canonical stored task record, run `orbit tool run orbit.task.show --input '{"id": "<returned-id>"}'`.

## Operating Rules

- Never edit task files directly.
- Never invent task IDs.
- `description` should be multi-line markdown when the task is non-trivial.
- Required fields: `title`, `description`, and `workspace`.
- Strongly prefer supplying `acceptance_criteria`.
- Blank or missing task companion files (`plan.md`, `execution-summary.md`) are treated as blank task fields. Repair them through `orbit.task.update` (`plan` or `execution_summary`), not manual file edits.
- Orbit fills `created_by`, `planned_by`, and `implemented_by` automatically from execution context when those roles are authored during the task lifecycle.
- Reserve task type `friction` for agent self-reports via `orbit-track-issues`. Do not use `friction` for normal task authoring.

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

```bash
orbit tool run orbit.task.add --input '{
  "title": "<title>",
  "description": "<multi-line markdown>",
  "acceptance_criteria": [
    "<observable outcome 1>",
    "<observable outcome 2>"
  ],
  "plan": "",
  "context": "<comma,separated,paths>",
  "workspace": "<absolute_or_relative_repo_path>",
  "priority": "<low|medium|high|critical>",
  "type": "<task|feature|issue|bug|chore|refactor>"
}'
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
