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

Submit backlog tasks or one or more named tasks through the gated shipment pipeline. The default mode opens PRs; `--mode local` uses the local-only delivery path. The command returns a run ID immediately, while dependency and lock waits happen inside the job.

```bash
orbit run ship
orbit run ship "$TASK_ID"
orbit run ship "$TASK_ID" "$SECOND_TASK_ID" --mode local
orbit run ship "$TASK_ID" --base main
```

Underlying job: `task_auto_pipeline`, which fans into `task_gate_pipeline` and then routes to `task_pr_pipeline` or `task_local_pipeline` from `--mode`.

## Completing work with `--complete`

By default a successful task ends in `review`, and a separate operator action
takes it to `done`. `--complete` is your explicit authorization, granted on one
invocation, for that run to finish delivery itself:

```bash
orbit run ship "$TASK_ID" --complete
orbit run auto --for 4h --complete
```

It is off unless you pass it. No workspace setting, environment variable, or
unattended routine (including `orbit run ship-sweep`) turns it on.

What the run then does depends on the mode:

- **`--mode local`** — the task reaches `done` only after the bundle has
  committed, merged, and pushed. A failed merge or push fails the run with the
  task still in `review`.
- **`--mode pr`** — the run opens or reuses the PR as usual, then merges it
  through GitHub. Branch protections and required checks are respected; Orbit
  never uses an administrative bypass. If required checks are still running it
  enables GitHub auto-merge and keeps waiting — enabling auto-merge is not
  success on its own. The task moves to `done` only after the PR is verified
  merged. A closed or blocked PR, a refused auto-merge, or an expired wait
  budget fails the run and leaves the task in `review`.
- **`no-diff-expected` work** — validated work that produced no diff completes
  without needing a PR.

Two limits are worth knowing:

- `orbit run auto --complete` is *blanket* authorization. It covers every task
  the drain admits for its whole window, including work that reaches the backlog
  after the run starts — not only what is visible when you submit.
- `--complete` authorizes delivery completion and the `review -> done`
  transition only. It never approves `proposed` work into the backlog, and it
  does not stand in for an independent review verdict; the transition is
  recorded against the authorizing run and operator in the task's history.

Submission stays asynchronous either way: the command prints the durable run ID
and returns without knowing the eventual outcome. Follow it with
`orbit run show <RUN_ID>`.

For an operator workflow that prepares pilot context, explicitly authorizes
backlog work, runs a bounded drain, and handles recovery, see [Run a Continuous
Delivery Window](../how-to/continuous-delivery/).

## Direct Job Execution

For schemaVersion 2 jobs without a workflow alias, invoke them directly:

```bash
orbit job list
orbit run job task_auto_pipeline
orbit run job task_auto_pipeline --input mode=local
orbit run job task_auto_pipeline --wait
```

A job run is submitted to a detached worker: the command prints the run ID and
returns as soon as the run is durable, without claiming its eventual outcome.
Pass `--wait` to block on the submitted run instead — it exits nonzero unless
the run succeeded.

## Inspecting Runs

Every workflow run is durable. Inspect with:

```bash
orbit run history -j task_auto_pipeline
orbit run show <RUN_ID>
orbit run logs <RUN_ID>
```
