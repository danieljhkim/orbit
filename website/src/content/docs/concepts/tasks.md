---
title: Tasks
description: "How Orbit models durable work, state transitions, acceptance criteria, and review."
sidebar:
  order: 2
---

## Definition

A task is a durable unit of work stored in workspace-local Orbit state. It carries a title, description, acceptance criteria, lifecycle state, context, review notes, and audit history.

Use tasks for work an agent can execute and a human can review. Do not use a task as a scratch note when no observable outcome exists.

## Lifecycle

The common path is:

```text
proposed -> backlog -> in-progress -> review -> done
```

Human-created direct tasks may enter the backlog immediately. Proposed tasks must be approved before normal execution. Review tasks are approved or rejected after the agent produces work.

### Statuses

| Status        | Purpose |
|---------------|---------|
| `proposed`    | Awaiting human approval before entering the backlog. |
| `friction`    | Agent self-reported friction awaiting triage. Creation-only — a task cannot return to `friction` after triage. |
| `backlog`     | Approved and queued for work. |
| `someday`     | Future-scoped — wanted but not yet actionable. Agents skip `someday` tasks. |
| `in-progress` | Actively being worked on. |
| `review`      | Implementation complete; awaiting review/merge. Requires an `execution_summary`. |
| `done`        | Accepted and closed. **Terminal** — no further transitions. |
| `blocked`     | Temporarily paused (waiting on a dependency or decision). |
| `archived`    | Soft-deleted via the dedicated `orbit task archive` command. Restorable to `backlog` with `orbit task update <id> --status backlog`. |
| `rejected`    | Declined. Can be re-opened to `backlog` or `in-progress`. |

### Transition rules

Transitions are permissive by default — any move is allowed unless it violates one of these invariants:

1. **Done is terminal.** No transitions out of `done`.
2. **Archived requires `orbit task archive`.** A bare `--status archived` update is rejected.
3. **Friction is creation-only.** A task that leaves `friction` cannot return.
4. **`in-progress → review` requires an `execution_summary`.**

## Quality Bar

A good task states:

- what should change
- where the change should happen
- how to observe success
- which files or selectors matter when known

Acceptance criteria should be testable. Prefer "command X exits successfully" or "file Y contains Z" over "the behavior feels better."
