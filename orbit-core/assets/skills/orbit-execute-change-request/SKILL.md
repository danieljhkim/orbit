---
name: orbit-execute-change-request
description: Execute a human-requested code change while managing the full lifecycle in Orbit tasks (create, update with execution comments, close). Use this before making any code changes, unless explicitly told not to.
---

# Execute Change Request

## Purpose

Handle human-initiated engineering changes (feature, refactor, improvement) from intent to verified implementation, with explicit task lifecycle tracking in `orbit task`.

---

## Required Task Lifecycle

This skill must manage a single Orbit task for each change request.

1. Create task at start.
2. Ensure task is approved before implementation or agent execution.
3. Update task during/after execution with implementation comments.
4. Close task when work is complete and validated.

Do not skip lifecycle updates.

---

## Inputs

- Natural-language change request
- Optional constraints (scope, files, deadlines)
- Repository workspace
- Optional owner / priority / task type hints

---

## Responsibilities

1. Clarify intent and success criteria
2. Create tracking task in Orbit
3. Obtain/record approval before execution
4. Plan minimal safe change
5. Implement modifications
6. Run validation (build/tests/lint)
7. Update task with execution comments and final status
8. Produce structured result

---

## Task Management Contract (`orbit task`)

### 1) Create Task (required before edits)

Use `orbit task add` with at least:

- `--title` from the change request
- `--description` summarizing requested outcome
- `--instructions` initial plan/constraints
- `--workspace` current repository path
- `--type` appropriate to request (`task|feature|issue|other`)

Example:

```bash
orbit task add \
  --title "Refactor task lifecycle handling" \
  --description "Apply requested refactor and preserve behavior" \
  --instructions "Implement minimal diff; run tests" \
  --workspace "/abs/path/to/repo" \
  --type feature \
  --priority medium
```

Capture and retain the created task ID.

### 2) Approve Task (required before implementation)

Preferred explicit approval:

```bash
orbit task approve <TASK_ID> --by "human" --note "approved verbally by user"
```

Flexible path when user gives explicit verbal approval in-session:

```bash
orbit agent run --task <TASK_ID> --approve-on-verbal --approved-by "agent" --approval-note "approved based on explicit user verbal confirmation"
```

If task approval is required by config (`[task.approval] required_for_agent = true`), execution must not proceed without one of the above.

### 3) Update Task With Execution Comments (required)

After implementation/validation, update the task with execution comments.

Because `orbit task` has no dedicated comment command, comments must be written into `--instructions` and/or `--description` as an execution summary.

Include:

- What changed
- Files touched
- Validation results
- Risks/follow-ups

Example:

```bash
orbit task update <TASK_ID> \
  --instructions "Execution comments: updated parser, added tests, build/tests pass." \
  --status in-progress
```

Then apply final update reflecting completion summary.

### 4) Close Task (required when done)

When acceptance criteria are met and validation is complete:

```bash
orbit task close <TASK_ID>
```

Verify final state:

```bash
orbit task show <TASK_ID>
```

---

## Execution Contract

- Operate only within the specified workspace.
- Prefer incremental, reviewable diffs.
- Preserve existing behavior unless explicitly changed.
- Fail fast on validation errors.
- Keep task state synchronized with actual execution state.


---

## Output

Persist the execution summary as a markdown file at:

```
~/.orbit/agents/<repo_name>/executions/YYYY-MM-DD-<title>.md
```

The execution summary file must include:

- Change request title
- Linked Orbit task ID
- Summary of implementation
- Files modified
- Validation results (build/tests/lint)
- Risks, follow-ups, and notes

Return output as markdown (not JSON), using this structure:

```markdown
# Execution Summary - <Change Request Title>

## Status
success | failed

## Orbit Task
Task ID: <orbit-task-id>

## Summary
<what changed>

## Files Modified
- <file path>

## Validation
- Build: pass | fail | skipped
- Tests: pass | fail | skipped
- Lint: pass | fail | skipped

## Notes
<execution comments, follow-ups, risks>
```

---

## Exit Criteria

- Requested change implemented
- Validation completed
- Task approved before execution
- Task updated with execution comments
- Task closed (`done`) when successful
- Markdown summary emitted and persisted
