---
type: runbook
summary: Diagnose, cancel, resume, or replay pending and running Orbit job runs.
tags: [operations, jobs, runs, recovery, debugging]
paths: ["crates/orbit-core/src/command/job/**", "crates/orbit-cli/src/command/run/**"]
related_features: [activity-job]
related_artifacts: [ORB-10070]
---

# Recover Stuck Job Runs

Use this runbook when a run remains `pending` or `running`, a worker dies, or a failed,
timed-out, or interrupted run needs to continue from its checkpoints.

## Inspect the run

`orbit job` manages job definitions plus resume and replay. `orbit run` is the run
observability surface.

```sh
orbit run history --limit 20        # recent runs; add -j <job_id> to filter
orbit run show   <run_id>           # per-step state table
orbit run logs   <run_id>           # raw stdout/stderr blobs (agent steps)
orbit run events <run_id>           # audit events for the run
orbit run trace  <run_id>           # parent/child event tree
```

Before changing run state, confirm the recorded owner PID and whether that exact process
is still alive. A long-running run is not necessarily stuck.

## Interpret run states

`pending → running → success | failed | timeout | cancelled | interrupted`
(plus transient `retrying` and step-level `skipped`).

`interrupted` means the run was orphaned: its owner process died through a crash, SIGKILL,
or reboot without finalizing the run. A job that genuinely failed is `failed`;
`interrupted` means the worker died.

## Understand orphan reconciliation

Every run records its owner `pid` plus a pid-start-time token. Pipeline workers claim their
queued run at startup, so `pending` runs carry an owner too [ORB-10070]. A reconcile pass
probes liveness and finalizes conclusively orphaned runs to `interrupted`, releasing their
task reservations:

- `running` runs with a dead owner;
- `pending` runs whose claimed worker died; and
- `pending` runs never claimed within a 30-minute grace window, such as queued children
  stranded when their parent run was interrupted by a reboot.

Reconciliation runs best-effort at workspace open and lazily on
`orbit run history` / `orbit run show`. `orbit doctor` reports orphans read-only. A run
whose PID is alive but unverifiable is deliberately left alone.

Example after a worker was SIGKILLed mid-step:

```text
$ orbit run history --limit 1
│ RUN_ID                 JOB_ID            ATTEMPT   STATE         ERROR_MESSAGE                              │
│ jrun-20260704-0927-2   demo_sleep_long   1         interrupted   job run marked interrupted because         │
│                                                                  recorded worker process is no longer alive │
│                                                                  (reason=process_not_found, pid=154953, …)  │
```

## Cancel a conclusively stuck run

After verifying that the owner is gone or that the run should no longer continue:

```sh
orbit run cancel <run_id>
```

This terminalizes the run on demand. Do not cancel solely because a legitimate step has
been `running` longer than expected.

## Resume from checkpoints

The v2 executor checkpoints every completed top-level step into
`job_runs.pipeline_state_json` in `~/.orbit/orbit.db`; there is no separate checkpoint
file. Resume accepts runs in `interrupted`, `failed`, or `timeout`. Any other state errors
with `resume requires an interrupted, failed, or timed-out run`.

```sh
orbit job resume <run_id>
```

Resume starts a new linked run with `attempt + 1` and `retry_source_run_id` set.
Checkpointed steps are skipped and their outputs are replayed into the pipeline:

```text
$ orbit job resume jrun-20260704-0927-2
$ orbit run events jrun-20260704-0928
│ 2026-07-04T09:28:15Z   -         run.started         cli:demo_sleep_long                                   │
│ 2026-07-04T09:28:15Z   nap_one   step.skipped        step=nap_one reason=resume: step already completed    │
│                                                      in checkpointed run (index 0)                         │
│ 2026-07-04T09:28:15Z   nap_two   step.started        nap_two                                               │
│ 2026-07-04T09:28:17Z   run.finished        success                                                         │
```

Resume needs the job present in the catalog (`orbit job list --all`). A run started from
a raw YAML path can be resumed only after that YAML is registered under `resources/jobs/`.
A run with no successful checkpoints degrades to a full replay.

## Replay from the beginning

When checkpoint outputs are invalid or the run must start from step zero:

```sh
orbit job replay <run_id>
```

## Verification

Use `orbit run show <new_run_id>` and `orbit run events <new_run_id>` to confirm the new
attempt is linked, expected completed steps were skipped on resume, and the run reached the
intended terminal state.

Related: [Inspect the audit trail](./audit-trail.md) ·
[Check Orbit health](./health-checks.md).
