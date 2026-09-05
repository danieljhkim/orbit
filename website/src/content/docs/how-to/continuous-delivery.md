---
title: Run a Continuous Delivery Window
description: "Prepare tasks, authorize a bounded backlog drain, inspect its runs, and recover safely."
sidebar:
  order: 6
---

Use this guide when you want a deliberate, time-bounded period of automatic delivery. It separates preparation, human approval, delivery, and recovery so an asynchronous run ID is never mistaken for a completed change.

## 1. Prepare proposed work

Start with the zero-input pilot. It discovers only `proposed` and `backlog`
tasks that have no `context_files`, then prepares bounded pilot groups.

```bash
orbit run job task_pilot_pipeline --wait
```

The pilot applies validated `context_files` selectors, but it does not approve
or dispatch work. Review both the pilot run and the task's applied selectors:

```bash
orbit run history -j task_pilot_pipeline
orbit run show <PILOT_RUN_ID>
orbit task show "$TASK_ID" --fields status,context_files
```

Use an explicit task list only when you intend to inspect those exact tasks;
the job input is a JSON array, not a space-separated list:

```bash
orbit run job task_pilot_pipeline --input 'task_ids=["TASK-123","TASK-456"]' --wait
```

## 2. Authorize the backlog

After reviewing a proposed task's pilot result, explicitly approve that task:

```bash
orbit task update "$TASK_ID" --approve --note "Pilot context reviewed for this delivery window."
```

That approval moves a `proposed` task to `backlog`. Pilot preparation itself
does not grant approval. If the task is already in `backlog`, do not approve it
again: once its pilot has applied the context you need, it is ready for the
next delivery run.

Dependencies and active locks are not bypassed by approval or by the drain.
They keep affected work in the backlog until it is eligible, so a submitted
window may legitimately leave some tasks unfinished.

## 3. Start a bounded delivery window

Choose the duration and whether this one run may complete delivery. The
following window lasts three hours and gives the run blanket `--complete`
authorization:

```bash
orbit run auto --for 3h --complete
```

`--complete` is opt-in authorization to move work from `review` to `done`; it
does not approve `proposed` tasks into the backlog. It applies to every task
the drain admits during this window, including a task admitted after the
command starts. Omit it when a separate operator should approve completion.

The command returns a durable parent run ID after submission, not a statement
that its tasks have completed. Record that ID as `<AUTO_RUN_ID>` and inspect
the parent and its child runs:

```bash
orbit run show <AUTO_RUN_ID>
orbit run trace <AUTO_RUN_ID>
orbit run show <CHILD_RUN_ID>
orbit run logs <CHILD_RUN_ID>
```

`orbit run trace` shows the run tree; use the child IDs it reports when a
particular task needs investigation. Also inspect the task directly to
distinguish its submitted run from its actual lifecycle state:

```bash
orbit task show "$TASK_ID" --fields status,job_run_id,comments
```

## 4. Recover from a failed delivery

First inspect the failed child run and its task. Fix a real code, review, or
dependency problem before creating or authorizing corrective work; do not
re-run blindly. For a task blocked by an attributable failed run, use bounded
triage to re-backlog only an environmental failure:

```bash
orbit run show <CHILD_RUN_ID>
orbit run logs <CHILD_RUN_ID>
orbit run triage "$TASK_ID"
orbit run history -j task_triage_pipeline
```

Triage never changes a task a human blocked by hand, and it leaves a
non-environmental diagnosis blocked for an operator decision. Inspect the
triage run before reopening the delivery window.

For task-state durability and same-authority recovery, use the [task
publication commands](../reference/cli/) and the [task-publication
runbook](https://github.com/danieljhkim/orbit/blob/main/docs/runbooks/task-publication.md).
For an operator working across hosts, the existing [federated MCP
setup](./mcp-integration/#register-the-federated-mux) explains how to select
the owning workspace without implying automatic failover.
