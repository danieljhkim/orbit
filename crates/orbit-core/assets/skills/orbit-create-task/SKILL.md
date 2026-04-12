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
6. Verify with `orbit tool run orbit.task.show --input '{"id": "<returned-id>"}'`.

## Operating Rules

- Never edit task files directly.
- Never invent task IDs.
- `description` should be multi-line markdown when the task is non-trivial.
- Required fields: `title`, `description`, and `workspace`.
- Strongly prefer supplying `acceptance_criteria`.
- Blank or missing task companion files (`plan.md`, `execution-summary.md`) are treated as blank task fields. Repair them through `orbit.task.update` (`plan` or `execution_summary`), not manual file edits.
- Orbit fills `created_by`, `assigned_to`, and `proposed_by` automatically from execution context.
- Reserve task type `friction` for agent self-reports via `orbit-track-issues`. Do not use `friction` for normal task authoring. Avoid task type `issue` for generic work too; it currently feeds the same scoreboard path.

## Task Quality Standards

### Environment Independence checklist

- Tests must not assume the presence of `.orbit/knowledge/`, local config files, or filesystem artifacts that are neither committed nor created by the test itself.
- File I/O tests must use temp directories or in-memory fakes.
- Acceptance criteria must state filesystem independence explicitly when behavior touches persisted state.

### Explicit Definitions checklist

- Terms such as `purpose`, `summary`, and `description` in acceptance criteria must include an observable format or output requirement.
- If acceptance criteria mention a `signature`, specify the exact form it must take.
- Prohibit vague pass/fail language such as `works correctly` or `handles edge cases`; replace it with concrete observable behavior.

### Mock-Based, Deterministic Testing

- Tasks that modify behavior involving external services, filesystem I/O, or time-dependent state must require mock or fake coverage in acceptance criteria.
- Prefer `node_type` attribute dispatch over `isinstance` checks when task code must remain compatible with mocks or fakes.
- Acceptance criteria must specify expected return types or output shapes when functions change.

### Per-Node Purpose checklist

- For graph or knowledge tasks, define each node `purpose` as: role in one sentence, crate or module, and whether the node is leaf or internal.

### AC Format Rule

- Each acceptance criterion must be independently verifiable and name a command, test, or observable output.
- Include at least one acceptance criterion that specifies the validation command.

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
  "type": "<task|feature|bug|chore|refactor>"
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
