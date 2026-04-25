---
title: Run a Task Lifecycle
description: "Create a task, approve it when needed, run it, and inspect the result."
sidebar:
  order: 2
---

## Create

```bash
TASK_ID=$(orbit task add \
  --title "Update docs for policy profiles" \
  --description "Document how fsProfile selection works for activity YAML." \
  --acceptance-criteria "The docs explain explicit and implicit fsProfile resolution." \
  --acceptance-criteria "The docs include a policy YAML example." \
  --workspace .)
```

## Inspect

```bash
orbit task show "$TASK_ID"
orbit task lint "$TASK_ID"
```

## Approve

If the task is proposed, approve it:

```bash
orbit task approve "$TASK_ID"
```

## Execute

```bash
orbit run ship "$TASK_ID"
```

Use local mode when PR creation is not desired:

```bash
orbit run ship --mode local "$TASK_ID"
```

## Review

Inspect the resulting diff, CI, task state, and audit events before approving a task out of review.

```bash
orbit task show "$TASK_ID"
orbit audit list
```
