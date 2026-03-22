---
name: orbit
description: Entry point for Orbit workflow. Covers lifecycle, invocation patterns, and skill routing. Load this for any Orbit-related work.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Orbit operations should go through the registered Orbit tool surface.

## Common Workflows

### Loading and executing a task

1. Load the task: `orbit tool run orbit.task.show --input '{"id": "<task-id>"}'`
2. Read the `plan` field — this is your implementation guide.
3. Read each file listed in `context_files` before making changes.
4. Start the task: `orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "..."}'`
5. Implement following the plan. Validate using the plan's verification steps.
6. Move to review: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review"}'`

### Reporting progress or problems

- Add a comment: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "what happened"}'`
- If execution fails, comment with what went wrong before stopping. The next agent needs this context.

### Finding work

- List backlog: `orbit tool run orbit.task.list --input '{"status": "backlog"}'`
- List in review: `orbit tool run orbit.task.list --input '{"status": "review"}'`

## Command Reference

Invoke Orbit through `orbit tool run`:

```bash
# Task commands
orbit tool run orbit.task.show --input '{"id": "<id>"}'              # Load task details
orbit tool run orbit.task.list --input '{"status": "backlog"}'       # List by status
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "plan": "...", "workspace": "."}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "..."}' # backlog -> in-progress
orbit tool run orbit.task.update --input '{"id": "<id>", "status": "review"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "comment": "..."}'
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "..."}'
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "..."}'

# Job run commands
orbit tool run orbit.job_run.list --input '{"status": "failed"}'
orbit tool run orbit.job_run.show --input '{"id": "<job_run_id>"}'
orbit tool run orbit.job_run.archive --input '{"id": "<job_run_id>"}'
```

Never edit task files directly.

## Lifecycle

```text
proposed → backlog → in-progress → review → done
```

Rejection path:

```text
proposed → rejected
review    → rejected
rejected  → backlog  (reconsider)
```

Use `blocked` when execution cannot safely continue.

Task commands infer actor provenance automatically:
- `orbit tool run ...` is treated as agent-driven
- direct `orbit task ...` CLI usage is treated as human-driven

## Skill Selection

- `orbit-create-task`: Create a new task with plan, context, and verification steps.
- `orbit-approve-task`: Record human approval or rejection at lifecycle gates.
- `orbit-execute-change-request`: Carry a change through implementation, validation, and review.
- `orbit-pr`: Creating PRs, leaving comments, replying to comment threads, and PR tool reference.
- `orbit-review-pr`: Review a pull request with per-issue comments and scoring compliance.
- `orbit-maintain-system`: Routine maintenance and operational job-run audits.
- `orbit-track-issues`: Capture discovered bugs, risks, or friction as tracked tasks.

## Voice Your Opinion

Orbit is built for agents. If something is unclear, missing, or creates friction, track it with `orbit-track-issues`.
