---
name: orbit-maintain-system
description: Perform routine, low-risk maintenance to keep the system healthy, consistent, and up-to-date without changing intended behavior. Use this skill only when explicitly requested.
---

# Orbit Maintain System

Use this skill for routine maintenance that is safe, incremental, and non-disruptive.

---

## Inputs
- Repository workspace
- Maintenance policy or schedule
- Optional scope constraints (paths, components)

---

## Responsibilities
1. Assess system health and identify maintenance issues.
2. Track every discovered issue via a new Orbit task.
3. Auto approve and apply low-risk maintenance tasks (deps, formatting, dead code, minor upgrades).
4. Verify integrity after changes (build/tests/lint as applicable).
5. Persist a markdown maintenance summary report.

---

## Assessment And Issue Tracking Contract

When assessment finds any issue (bug, risk, drift, failing check, deprecated usage, security concern), create a tracking task immediately via `orbit task add` - refer to `orbit-create-task` skill on how to create a task.

For canonical `orbit task` CLI workflows (update/show/search), refer to the `orbit-manage-tasks` skill.

This skill's issue-tracking requirements:

- Create one Orbit task per issue discovered
- Use `--type issue` for defects/risks; use `--type chore` only when clearly non-defect maintenance work.
- Map severity to `--priority` (`high`, `medium`, `low`).
- Record created task IDs in the final report.

If no issues are found, explicitly state "No maintenance issues found" in the report.

---

## Execution Contract
- Preserve observable behavior
- Prefer minimal, incremental changes
- Avoid breaking API or schema contracts
- Abort on validation failure
- Produce reviewable diffs
- Do not silently ignore discovered issues; track them with Orbit tasks

---

## Output

Persist a markdown report to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD-<title>.md`

If a maintenance artifact belongs to exactly one Orbit task, store that task-owned artifact under the linked task bundle's `artifacts/` directory instead of `agents/reports/`.

The report must use below template:

## Report Template

```markdown
# Maintenance Summary - <Title>

Agent Name: <agent name>
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
- <TASK_ID> - <title> (priority: <low|medium|high>, status: <backlog|in_progress|done>)

## Notes
<follow-ups, risks, blockers>
```

---

## Exit Criteria
- Assessment completed
- Every discovered issue is tracked via newly created Orbit task(s)
- Maintenance actions completed or safely skipped
- Validation completed
- Markdown report written to `{{ORBIT_ROOT}}/agents/reports/YYYY-MM-DD-<title>.md` unless it is a single-task artifact, in which case it belongs under that task bundle's `artifacts/`
- Response includes report location and outcome summary
