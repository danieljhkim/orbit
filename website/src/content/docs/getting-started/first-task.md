---
title: First Task
description: "Create an Orbit task, inspect it, and run the default task shipping flow."
sidebar:
  order: 3
---

## Create a Task

Tasks are the durable unit of work. Create one with concrete acceptance criteria.

```bash
TASK_ID=$(orbit task add \
  --title "Create orbit-hello.txt" \
  --description "Add orbit-hello.txt at the repository root containing the text 'hello from orbit'." \
  --acceptance-criteria "orbit-hello.txt exists at the repository root." \
  --acceptance-criteria "orbit-hello.txt contains the text 'hello from orbit'." \
  --workspace .)

echo "$TASK_ID"
```

## Inspect It

```bash
orbit task list
orbit task show "$TASK_ID"
orbit task lint "$TASK_ID"
```

If the task entered a proposal state, approve it before execution:

```bash
orbit task approve "$TASK_ID"
```

## Ship It

Run the default PR-based path:

```bash
orbit run ship "$TASK_ID"
```

Run local-only when you do not want Orbit to open or update a pull request:

```bash
orbit run ship --mode local "$TASK_ID"
```

## Pin Multiple Tasks

When you already know the work set, pin task IDs explicitly:

```bash
orbit run ship T123 T456 --base main
```
