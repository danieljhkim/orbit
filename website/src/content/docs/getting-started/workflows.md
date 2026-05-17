---
title: Default Workflows
description: "Built-in workflows under orbit run: ship and duel-plan."
sidebar:
  order: 4
---

Orbit ships two default workflows under `orbit run`. Each wraps a seeded job pipeline under `crates/orbit-core/assets/jobs/`; the same pipelines are runnable directly via `orbit run job <name>`.

All three workflows default `--base` to `agent-main` — the convention for agent-targeted long-lived branches that humans merge into `main` after review. Pass `--base <branch>` to target a different branch.

## `orbit run ship`

Submit backlog tasks or one or more named tasks through the gated shipment pipeline. The default mode opens PRs; `--mode local` ships in-place. The command returns a run ID immediately, while dependency and lock waits happen inside the job.

```bash
orbit run ship
orbit run ship T20260506-1
orbit run ship T20260506-1 T20260506-2 --mode local
orbit run ship T20260506-1 --base main
```

Underlying job: `task_auto_pipeline`, which fans into `task_gate_pipeline` and then routes to `task_pr_pipeline` or `task_local_pipeline` from `--mode`.

## `orbit run duel-plan`

Run a planning duel for a single task: two planner agents draft proposals independently, an arbiter picks the winner, and the winning plan lands on the task.

```bash
orbit run duel-plan T20260506-1
orbit run duel-plan T20260506-1 --base main --json
```

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
orbit run show <RUN_ID>
orbit run logs <RUN_ID>
```

## Choosing a Backend

**v1 supports `backend: cli` only.** For `agent_loop` activities, backend resolution follows: `--backend` → `ORBIT_BACKEND` → `[runtime] backend` in config → hard-coded fallback `http`. Pin `cli` explicitly until v2:

```bash
orbit run ship T20260506-1 --backend cli
```

`backend: http` is wired but not part of the v1 release surface — treat it as preview only.
