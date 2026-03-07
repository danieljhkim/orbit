---
name: orbit-execute-change-request
description: Use this when executing human-initiated code change in order to manage the full lifecycle in Orbit tasks (create, update, archive). Use this before and after making any code changes.
---

# Orbit Execute Change Request

## Purpose

Handle human-initiated engineering changes (feature, refactor, improvement, issue) from intent to verified implementation, with explicit task lifecycle tracking in `orbit task`.

---

## Inputs

- Natural-language change request
- Constraints (scope, files, deadlines), if any
- Repository workspace
- Priority/type hints, if any
- Actor identity metadata (display name), if available

---

## Responsibilities

1. Clarify intent and success criteria.
2. Create or link the tracking task in Orbit.
3. Obtain/record approval before execution (if task is in `proposed` status). If approval cannot be obtained right away, your job is done for now.
4. Implement the requested change and validate.
5. Once the task is completed,
6. Persist the execution summary in the linked task bundle.

---

## Required Task Lifecycle

For the canonical `orbit task` CLI workflows (create/update/search/approve/archive), refer to the `orbit-manage-tasks` skill. It defines required attribution fields and post-mutation verification.

Manage a SINGLE Orbit task per change request:

1. Create task at start, if task is not already created.
2. If any doubt remains, ask clarifying questions and record them in the task comments.
3. Ensure task is approved before implementation (if `proposed`). If approval cannot be obtained right away, your job is done for now.
4. Update task status (from `backlog`) to `in_progress` before execution.
5. Once change is completed, update the status from `in_progress` to `review`
6. 

Do not skip lifecycle updates.

---

## Output

Persist the execution summary markdown in the linked task bundle at:

```
{{ORBIT_ROOT}}/tasks/<current-status>/<task-id>/execution-summary.md
```

If you need additional task-owned notes beyond the canonical execution summary, place them under:

```
{{ORBIT_ROOT}}/tasks/<current-status>/<task-id>/artifacts/
```

The execution summary file must include:

- Change request title
- Linked Orbit task ID
- Summary of implementation
- Files modified
- Validation results (build/tests/lint)
- Risks, follow-ups, and notes

Return output as markdown, using this structure:

```markdown
# Execution Summary - <Change Request Title>

Agent Name: <agent name>
Agent Model: <model name>

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
- Task approved before execution (if required)
- Task updated with execution comments
- Task archived when successful
- Markdown summary written to the linked task bundle's `execution-summary.md`
