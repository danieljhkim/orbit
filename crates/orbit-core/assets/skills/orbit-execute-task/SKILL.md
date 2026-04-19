---
name: orbit-execute-task
description: Use this when executing an existing Orbit task or carrying a human request through the Orbit task lifecycle with explicit status tracking.
---

# Orbit Execute Task

## Purpose

Handle a human-requested engineering task or existing Orbit task from intent to verified implementation, with explicit task lifecycle tracking.

## Command Reference

All agent Orbit interactions go through `orbit tool run`. Never use `orbit task ...` directly — it skips agent provenance. Never guess tool names — run `orbit tool list` to see all registered tools.

When invoking `orbit tool run` directly, include `agent` and `model` in the input JSON:

```json
{
  "agent": "<claude|codex|gemini>",
  "model": "<model_name>"
}
```

```bash
# Load a full task
orbit tool run orbit.task.show --full --input '{"id": "<task-id>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'

# Load a specific field only
orbit tool run orbit.task.show --input '{"id": "<task-id>", "field": "plan", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
# Valid fields: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts

# Start a task (proposed/backlog/someday/blocked -> in-progress)
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'

# Update plan, status, or add a comment
orbit tool run orbit.task.update --input '{"id": "<task-id>", "plan": "<markdown plan>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "<what happened>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'

# Persist execution summary
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'

# List tasks
orbit tool run orbit.task.list --input '{"status": "backlog", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

**If running from a worktree**, pass `--root` pointing to the original repo's `.orbit` directory so commands resolve correctly.

## Workflow

### Step 1: Load or create the task

**If given an existing task ID**, load it with `orbit.task.show`. Extract:
- `description` and `acceptance_criteria` — these define the required outcome.
- `plan` — if blank or placeholder, author a plan before starting.
- `context_files` — read each file before making changes.
- `status` — confirm the task is ready to start.

**If this is a new task** (no task ID), clarify intent and success criteria with the human, then create via `orbit-create-task`.

### Step 2: Plan

If the task lacks a concrete plan, write one:

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "plan": "<markdown plan>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

Replace placeholders like `To be authored by executing agent at start time.` Keep the plan concrete: target files, validation commands, risks.

### Step 3: Start

```bash
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why this is ready>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

- Moves `backlog -> in-progress` or `proposed -> in-progress` (records approval automatically).
- Starting from `proposed` still requires a real plan; starting from `backlog` does not.
- Use explicit `approve` + later status updates when approval and execution should stay separate.

### Step 4: Implement and validate

Follow the task's `plan` step by step. Read `context_files` before touching code. Run the repo-approved verification commands from the plan. If repo instructions forbid tests, honor that and use the allowed validation path instead.

### Step 5: Move to review and summarize

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

Then persist the execution summary (see template below):

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>", "agent": "<claude|codex|gemini>", "model": "<model_name>"}'
```

## Execution Summary Template

Required sections:

```markdown
## Status
success | failed

## Summary of Changes
<what changed and why>

## Overall Assessment
<short quality assessment>
```

Include when relevant (omit if N/A):

```markdown
## Strategic Decisions
- <decision> | Rationale: <why>

## Design Weaknesses / Risks
- <risk> | Severity: Low / Medium / High | Mitigation: <mitigation>

## Deviations from Original Plan
- <deviation> | Justification: <why>

## Recommended Follow-Ups
- <next step>
```

## Lifecycle Rules

- One Orbit task per activity invocation. Do not multiplex tasks.
- If material ambiguity remains, ask clarifying questions before implementation.
- If approval cannot be obtained for `proposed` work, stop after recording that state.
- Do not skip lifecycle updates.
- Moving `in-progress -> review` requires a non-empty `execution_summary`, so persist the summary before or together with the review transition.

## Exit Criteria

- Requested change implemented and validated.
- Task started via `orbit.task.start` before execution.
- Task advanced to `review`.
- Execution summary persisted via `orbit.task.update`.
