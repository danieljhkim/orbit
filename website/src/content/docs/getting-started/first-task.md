---
title: First Task
description: "Create an Orbit task, inspect it, and run the default task shipping flow."
sidebar:
  order: 3
---

## Create a Task

Tasks are the durable unit of work. Create one with concrete acceptance criteria.

Tasks require explicit acceptance criteria because agents use them to self-evaluate completion and determine when their work is actually done.

```bash
TASK_ID=$(orbit task add \
  --title "Create orbit-hello.txt" \
  --description "Add orbit-hello.txt at the repository root containing the text 'hello from orbit'." \
  --acceptance-criteria "orbit-hello.txt exists at the repository root." \
  --acceptance-criteria "orbit-hello.txt contains the text 'hello from orbit'." \
  --workspace .)

echo "$TASK_ID"

# or ask an agent to create a task
"create an orbit task for ..."
```

## Inspect It

```bash
orbit task list
orbit task show "$TASK_ID"
orbit task lint "$TASK_ID"
```

If the task entered a proposal state, approve it before execution:

```bash
orbit task update "$TASK_ID" --status backlog
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
