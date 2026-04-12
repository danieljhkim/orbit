---
name: orbit-execute-change-request
description: Use this when executing human-initiated code change or existing orbit task in order to manage the full lifecycle in Orbit tasks (create, update, archive). Use this when the user specifically instructs you to use "orbit skill".
---

# Orbit Execute Change Request

## Purpose

Handle a human-requested engineering change or existing Orbit task from intent to verified implementation, with explicit task lifecycle tracking.

## Command Reference

All agent Orbit interactions go through `orbit tool run`. **Never guess command names — use exactly these:**

Include your identity on every `orbit tool run orbit.*` call by passing `agent` and `model` in the input JSON. Orbit uses these fields to write precise task provenance (`history`, `assigned_to`, comments, and task metadata) instead of the generic `agent` label.

```bash
# Load a task
orbit tool run orbit.task.show --input '{"id": "<task-id>", "agent": "<agent>", "model": "<model>"}'

# Start a task (backlog/proposed -> in-progress)
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why>", "agent": "<agent>", "model": "<model>"}'

# Update task status
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "agent": "<agent>", "model": "<model>"}'

# Update execution summary
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>", "agent": "<agent>", "model": "<model>"}'

# Add a comment
orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "<what happened>", "agent": "<agent>", "model": "<model>"}'

# List tasks
orbit tool run orbit.task.list --input '{"status": "backlog", "agent": "<agent>", "model": "<model>"}'
```

**Important:** Always use `orbit tool run` — never use `orbit task ...` directly. The tool interface tracks agent provenance. Direct CLI usage is reserved for humans.

**If running from a worktree**, pass `--root` pointing to the original repo's `.orbit` directory so commands resolve correctly.

## Common Mistakes — DO NOT

Agents frequently make these mistakes. **Read this before running any command.**

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run orbit.task.show ...` | `cargo run` rebuilds from source; agents must use the installed binary | `orbit tool run orbit.task.show ...` |
| `orbit task show ...` | Direct CLI subcommands are for humans; agent provenance is not recorded | `orbit tool run orbit.task.show ...` |
| `orbit tool run orbit.task.transition ...` | `orbit.task.transition` does not exist — there is no such tool | `orbit tool run orbit.task.start ...` or `orbit tool run orbit.task.update ...` |
| `orbit tool run orbit.task.move ...` | `orbit.task.move` does not exist | `orbit tool run orbit.task.update --input '{"id":"...","status":"..."}'` |
| `orbit tool run orbit.task.comment ...` | `orbit.task.comment` does not exist | `orbit tool run orbit.task.update --input '{"id":"...","comment":"..."}'` |

**Rule:** If a tool name is not listed in the Command Reference above, it does not exist. Never invent tool names. Run `orbit tool list` to see all registered tools.

## Workflow

### Step 1: Load or create the task

**If given an existing task ID**, load it:

```bash
orbit tool run orbit.task.show --input '{"id": "<task-id>", "agent": "<agent>", "model": "<model>"}'
```

Read the returned JSON carefully. Extract:
- `description` and `acceptance_criteria` — these define the required outcome.
- `plan` — if blank or placeholder text, author a fresh execution plan before starting work.
- `context_files` — read each file listed before making changes.
- `status` — confirms the task is ready to start.

**If this is a new change request** (no task ID), clarify intent and success criteria with the human, then create a task using `orbit-create-task`.

### Step 2: Plan the task

If the task does not already have a real, current plan, write one first:

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "plan": "<markdown plan>", "agent": "<agent>", "model": "<model>"}'
```

- Use the task description and acceptance criteria as the source of truth.
- Replace placeholder plans like `To be authored by executing agent at start time.`
- Keep the plan concrete: likely files, validation commands, and major risks.

### Step 3: Start the task

```bash
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why this is ready now>", "agent": "<agent>", "model": "<model>"}'
```

- `task start` moves `backlog -> in-progress`
- `task start` also handles `proposed -> in-progress` and records `proposal_approved` before `started`
- `task start` still requires a real plan when jumping directly from `proposed -> in-progress`; backlog tasks may start with an empty plan
- Keep using explicit `approve` plus later status updates when approval and execution should stay separate

### Step 4: Implement and validate

Follow the task's `plan` field step by step. Read the `context_files` before touching code. If the plan has verification commands, run them.

### Step 5: Move to review

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "agent": "<agent>", "model": "<model>"}'
```

### Step 6: Persist the execution summary

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
  "execution_summary": "<summary content>",
  "agent": "<agent>",
  "model": "<model>"
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

## Exit Criteria

- Requested change implemented
- Validation completed
- Task started via `orbit tool run orbit.task.start` with explicit `agent` and `model` before execution
- Task advanced through `review`
- Execution summary persisted via `orbit tool run orbit.task.update --input '{"id": "...", "execution_summary": "...", "agent": "...", "model": "..."}'`
