---
title: Default Workflows
description: "Built-in workflows under orbit run."
sidebar:
  order: 4
---

Orbit ships a default workflow under `orbit run`. It wraps a seeded job pipeline under `crates/orbit-core/assets/jobs/`; the same pipeline is runnable directly via `orbit run job <name>`.

The workflow defaults `--base` to `[workflow].base_branch` from
`config.toml`, or `main` when it is unset. Pass `--base <branch>` to target a
different branch.

## `orbit run ship`

Submit backlog tasks or one or more named tasks through the gated shipment pipeline. The default mode opens PRs; `--mode local` ships in-place. The command returns a run ID immediately, while dependency and lock waits happen inside the job.

```bash
orbit run ship
orbit run ship "$TASK_ID"
orbit run ship "$TASK_ID" "$SECOND_TASK_ID" --mode local
orbit run ship "$TASK_ID" --base main
```

Underlying job: `task_auto_pipeline`, which fans into `task_gate_pipeline` and then routes to `task_pr_pipeline` or `task_local_pipeline` from `--mode`.

## Direct Job Execution

For schemaVersion 2 jobs without a workflow alias, invoke them directly:

```bash
orbit job list
orbit run job task_auto_pipeline
orbit run job task_auto_pipeline --input mode=local
```

## Inspecting Runs

Every workflow run is durable. Inspect with:

```bash
orbit run history -j task_auto_pipeline
orbit run show <RUN_ID>
orbit run logs <RUN_ID>
```
