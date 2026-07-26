---
title: Default Workflows
description: "Built-in workflows under orbit run: ship and duel-plan."
sidebar:
  order: 4
---

Orbit ships two default workflows under `orbit run`. Each wraps a seeded job pipeline under `crates/orbit-core/assets/jobs/`; the same pipelines are runnable directly via `orbit run job <name>`.

Both workflows default `--base` to `[workflow].base_branch` from
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

## `orbit run duel-plan`

Submit a planning duel for a single task: two planner agents draft proposals independently, an arbiter picks the winner, and the winning plan lands on the task. The command returns a run ID immediately by default; pass `--wait` when you want the terminal to block until the duel finishes and report the terminal wait status.

```bash
orbit run duel-plan "$TASK_ID"
orbit run duel-plan "$TASK_ID" --base main --json
orbit run duel-plan "$TASK_ID" --wait
```

Default text output includes `Workflow`, `Job ID`, `Run ID`, `State`, and an `Inspect:` command. JSON output returns the submitted dispatch result with `workflow`, `job_id`, `run_id`, `state`, and `attempt` fields.

Underlying job: `job_duel_plan_pipeline`. Outcomes are recorded on the planning-duel scoreboard.

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
orbit run history -j job_duel_plan_pipeline
orbit run show <RUN_ID>
orbit run logs <RUN_ID>
```
