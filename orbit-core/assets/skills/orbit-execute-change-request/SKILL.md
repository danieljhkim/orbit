---
name: orbit-execute-change-request
description: Use this when executing human-initiated code change or existing orbit task in order to manage the full lifecycle in Orbit tasks (create, update, archive). Use this when the user specifically instructs you to use "orbit skill".
---

# Orbit Execute Change Request

## Purpose

Handle a human-requested engineering change or existing Orbit task from intent to verified implementation, with explicit task lifecycle tracking.

## Command Reference

All agent Orbit interactions go through `orbit tool run`. **Never guess command names — use exactly these:**

```bash
# Load a task
orbit tool run orbit.task.show --input '{"id": "<task-id>"}'

# Start a task (backlog/proposed -> in-progress)
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why>"}'

# Update task status
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review"}'

# Update execution summary
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>"}'

# Add a comment
orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "<what happened>"}'

# List tasks
orbit tool run orbit.task.list --input '{"status": "backlog"}'
```

**Important:** Always use `orbit tool run` — never use `orbit task ...` directly. The tool interface tracks agent provenance. Direct CLI usage is reserved for humans.

**If running from a worktree**, pass `--root` pointing to the original repo's `.orbit` directory so commands resolve correctly.

## When Commands Fail

If any `orbit tool run` command fails unexpectedly (unknown tool, missing field, unclear error), **do not silently work around it**. Immediately create a friction task:

```bash
orbit tool run orbit.task.add --input '{"title": "<short problem statement>", "description": "<what command failed, the error message, and why it caused friction>", "type": "issue", "priority": "medium"}'
```

Then continue with your work. The friction must be recorded so it gets fixed for the next agent.

## Workflow

### Step 1: Load or create the task

**If given an existing task ID**, load it:

```bash
orbit tool run orbit.task.show --input '{"id": "<task-id>"}'
```

Read the returned JSON carefully. Extract:
- `plan` — this is your implementation guide. Follow its steps.
- `context_files` — read each file listed before making changes.
- `description` — the problem statement and success criteria.
- `status` — confirms the task is ready to start.

**If this is a new change request** (no task ID), clarify intent and success criteria with the human, then create a task using `orbit-create-task`.

### Step 2: Start the task

```bash
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why this is ready now>"}'
```

- `task start` moves `backlog -> in-progress`
- `task start` also handles `proposed -> in-progress` and records `proposal_approved` before `started`
- Keep using explicit `approve` plus later status updates when approval and execution should stay separate

### Step 3: Implement and validate

Follow the task's `plan` field step by step. Read the `context_files` before touching code. If the plan has verification commands, run them.

### Step 4: Move to review

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review"}'
```

### Step 5: Persist the execution summary

See Output section below.

## Lifecycle Rules

- Manage a single Orbit task per change request.
- If material ambiguity remains, ask clarifying questions before implementation.
- If approval cannot be obtained for `proposed` work, stop after recording that state instead of calling `task start`.
- Do not skip lifecycle updates.

## Output

Persist the execution summary via the Orbit task update tool — do NOT write to a file path directly:

```bash
orbit tool run orbit.task.update --input '{
  "id": "<task-id>",
  "execution_summary": "<summary content>"
}'
```

The Orbit task tool resolves the correct bundle path automatically. Never hardcode or guess the file location.

Use this structure for the execution summary:

```markdown
# Execution Summary - <Change Request Title>
Agent Name: <agent_name>
Agent Model: <model name>

## Status
success | failed

## Orbit Task
Task ID: <orbit-task-id>

## 1. Summary of Changes
<what changed>
## 2. Strategic Decisions
- <decision> | Rationale: <why> | Trade-offs: <trade-offs>
## 3. Assumptions Made
- <assumption> | Impact if incorrect: <impact>
## 4. Design Weaknesses / Risks
- <risk> | Severity: Low / Medium / High | Mitigation: <mitigation>
## 5. Deviations from Original Plan
- <deviation> | Justification: <why>
## 6. Technical Debt Introduced
- <item> | Recommended resolution: <next step>
## 7. Recommended Follow-Ups
- <next step>
## 8. Overall Assessment
<short quality assessment>
```

## Responding to PR Review Comments

See `orbit-pr` skill for the full PR tool reference. Key rules for implementers:
- **Push back with reasoning when you disagree.** Last comment wins — if you stay silent, the reviewer's comment stands as the verdict.
- **If the reviewer is right, fix the code and reply the confirmation** Don't just silently push a commit.


## Exit Criteria

- Requested change implemented
- Validation completed
- Task started via `orbit tool run orbit.task.start` before execution
- Task advanced through `review`
- Execution summary persisted via `orbit tool run orbit.task.update --input '{"id": "...", "execution_summary": "..."}'`
