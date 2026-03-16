---
name: orbit
description: Entry point for Orbit workflow. Covers lifecycle, invocation patterns, identity, and skill routing. Load this for any Orbit-related work.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Orbit operations should go through the registered Orbit tool surface.

## Invocation Patterns

Invoke Orbit through `orbit tool run`:

```bash
orbit tool run orbit.task.list --input '{"status": "backlog"}'
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "plan": "...", "workspace": "."}'
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "<note>"}'
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "<note>"}'
orbit tool run orbit.task.show --input '{"id": "T20260315-123456"}'
orbit tool run orbit.task.update --input '{"id": "T20260315-123456", "status": "review"}'
orbit tool run orbit.task.update --input '{"id": "T20260315-123456", "comment": "notes here"}'
orbit tool run orbit.identity.list --input '{"role": "engineer"}'
orbit tool run orbit.identity.show --input '{"id": "<identity_id>"}'
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

## Identity

Run `orbit tool run orbit.identity.list --input '{"role": "engineer"}'` (or another role), then `orbit tool run orbit.identity.show --input '{"id": "<id>"}'`. Assume this identity for the session.

Task commands infer actor provenance automatically:
- `orbit tool run ...` is treated as agent-driven
- direct `orbit task ...` CLI usage is treated as human-driven
- activity-driven executions use the activity `identity_id` when available

## Skill Selection

- `orbit-create-task`: Create a new task with plan, context, and verification steps.
- `orbit-approve-task`: Record human approval or rejection at lifecycle gates.
- `orbit-execute-change-request`: Carry a change through implementation, validation, and review.
- `orbit-maintain-system`: Routine maintenance and operational job-run audits.
- `orbit-track-issues`: Capture discovered bugs, risks, or friction as tracked tasks.

## Voice Your Opinion

Orbit is built for agents. If something is unclear, missing, or creates friction, track it with `orbit-track-issues`.
