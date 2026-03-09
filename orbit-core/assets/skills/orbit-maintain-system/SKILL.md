---
name: orbit-maintain-system
description: Perform routine, low-risk maintenance to keep the system healthy, consistent, and up-to-date without changing intended behavior. Use this skill only when explicitly requested.
---

# Orbit Maintain System

## Purpose

Use this skill for explicitly requested maintenance that is safe, incremental, and does not change intended behavior.

## Responsibilities

1. Assess system health and identify maintenance issues.
2. Track every discovered issue via a new Orbit task.
3. Apply low-risk maintenance such as formatting, dead code removal, small dependency updates, or minor cleanup.
4. Verify integrity after changes with build/tests/lint as applicable.
5. Persist a markdown maintenance summary report.

## Issue Tracking Contract

When maintenance finds any issue, create a tracking task immediately.

- Create one Orbit task per issue discovered.
- Use `--type issue` for defects/risks and `--type chore` only for clearly non-defect maintenance work.
- Map severity to `--priority` (`high`, `medium`, `low`).
- Record created task IDs in the final report.
- If no issues are found, say "No maintenance issues found".

Use `orbit-create-task` for task creation details and `orbit-manage-tasks` for update/show/search flows.

## Execution Rules

- Preserve observable behavior.
- Prefer minimal, incremental changes.
- Avoid breaking API or schema contracts.
- Abort on validation failure.
- Produce reviewable diffs.
- Do not silently ignore discovered issues.

## Output

Write the report to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD-<title>.md`.

If an artifact belongs to exactly one Orbit task, store it under that task bundle's `artifacts/` directory instead.

Use this structure:

```markdown
# Maintenance Summary - <Title>
Agent Name: <identity_display_name>
Agent Model: <model name>

## Status
success | failed
## Scope
<paths/components reviewed>
## Actions Performed
- <action>
## Files Modified
- <file path>
## Validation
- Build: pass | fail | skipped
- Tests: pass | fail | skipped
- Lint: pass | fail | skipped
## Issues Found
- <issue summary>
## Orbit Tasks Created
- <TASK_ID> - <title> (priority: <low|medium|high>, status: <status>)
## Notes
<follow-ups, risks, blockers>
```

## Exit Criteria

- Assessment completed
- Every discovered issue tracked
- Maintenance actions completed or safely skipped
- Validation completed
- Report written to the correct location
