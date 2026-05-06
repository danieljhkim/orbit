---
title: Default Workflows
description: "Built-in workflows under orbit run: ship, ship-auto, and duel-plan."
sidebar:
  order: 4
---

Orbit ships three default workflows under `orbit run`. Each wraps a seeded job pipeline under `crates/orbit-core/assets/jobs/`; the same pipelines are runnable directly via `orbit run job <name>`.

All three workflows default `--base` to `agent-main` — the convention for agent-targeted long-lived branches that humans merge into `main` after review. Pass `--base <branch>` to target a different branch.

## `orbit run ship`

Carry one or more named tasks from `backlog` through `done`. The default mode opens a PR; `--mode local` ships in-place.

```bash
orbit run ship T20260506-1
orbit run ship T20260506-1 T20260506-2 --mode local
orbit run ship T20260506-1 --base main
```

Underlying job: `task_pr_pipeline` (PR mode) or `task_local_pipeline` (`--mode local`).

## `orbit run ship-auto`

Pick eligible tasks from the backlog, group them into bundles, and run each bundle through the same gate pipeline as `ship`. No task IDs are required.

```bash
orbit run ship-auto
orbit run ship-auto --mode local
orbit run ship-auto --base main
```

Output statuses: `empty_backlog`, `gated_noop`, `gate_waiting`, `gate_failed`, `completed`. Gated and no-op statuses keep exit code 0 and are reported explicitly in both text and `--json` output.

Underlying job: `task_auto_pipeline`.

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
