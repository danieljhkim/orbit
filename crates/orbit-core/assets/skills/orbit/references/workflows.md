# Jobs, activities, and runs

Orbit's execution layer. This covers the mechanics; for deciding *what* to
dispatch see [orchestration.md](orchestration.md), and for scheduling it see
[automation.md](setup/automation.md).

## Concepts

- **Job** — a deterministic, multi-step pipeline (schemaVersion 2 YAML) from the
  installed catalog. Jobs compose activities. Discover with `orbit job list` /
  `orbit job show <id>`.
- **Activity** — one named step definition referenced by a job's step list
  (`agent_implement`, `task_pilot`, `git_commit`, `git_push`, `pr_open`,
  `worktree_setup`, `reserve_locks`, ...). Activities are never invoked directly
  by CLI; a job's step list references them. `orbit activity list` shows the
  catalog.
- **Run** — one execution, with a `jrun-*` id, a durable state bundle under
  `.orbit/state/job-runs/`, and an audit trail.

For every task-backed agent activity, Orbit computes
`effective_tools = deduplicate(activity.tools union task.required_tools)`. The
activity list remains the baseline; an empty task requirement list preserves it
exactly. When one agent activity selects a batch, Orbit unions the requirements
from every selected task into that same effective list. Admission rejects
invalid required names before provider launch, and
the run envelope, `ORBIT_ACTIVITY_TOOLS`, and audit evidence carry the effective
list. Tool inclusion does not bypass later role, capability, policy, sandbox,
subprocess, or authentication checks.

## Running a job

```bash
orbit job list                                   # catalog
orbit job show <job_id>
orbit run job <job_id> --input key=value --json
orbit run job <job_id> --wait                    # block until terminal; nonzero unless it succeeded
orbit run history --json
orbit run history -j <job_id>
orbit run show <run_id> --json
```

**Runs are asynchronous by default.** `orbit run job` submits to a detached
worker and returns as soon as the run is durable — it prints the run id and the
inspection commands, and does *not* claim the eventual outcome. Add `--wait` to
block on it.

Equivalent catalog commands exist as `orbit job list|show|run|replay|resume`.
`replay` re-runs from step 0 against the current definition; `resume` continues
an interrupted run from its persisted step checkpoints, skipping completed
steps.

## The shipped pipelines

| Job | Purpose |
|---|---|
| `task_pr_pipeline` | Implement a task in a worktree and open a PR. |
| `task_local_pipeline` | Same, committing to the current branch without a PR. |
| `task_auto_pipeline` | Discover ready backlog tasks and ship them. |
| `task_gate_pipeline` | Gated shipment with windowing and starvation handling. |
| `task_pilot_pipeline` | Read-only preflight that fills validated `context_files`. |
| `task_triage_pipeline` | Diagnose tasks blocked by failed runs. |
| `epic_pipeline` | Ship an epic and its descendants against one worktree. |
| `workspace_ship_pipeline` / `workspace_auto_pipeline` | Workspace-scoped wrappers that resolve mode and base branch, then invoke the pipelines above. |
| `auto_task_scheduler_pipeline` | Mint tasks from due auto-task definitions. |
| `worktree_gc_pipeline` | Reclaim settled worktrees. |

Inspect any of them with `orbit job show <id>` before invoking — the step list is
the contract.

## Cancelling

```bash
orbit run cancel <run_id>
```

For a run that is stuck rather than merely slow, diagnose before killing:
[run-debugging.md](run-debugging.md) covers matching a run id to its process
group and the safe termination order.

## Diagnosis

- A `jrun-*` id that failed, stuck, or was cancelled →
  [run-debugging.md](run-debugging.md) for the full flow: run bundle, audit
  trail, logs and blobs, failure classification, task and git state, kill
  procedure, report format.
- A known failure signature, once the failing step is identified →
  [common-failures.md](common-failures.md).
- Host-level incident, service warning, or missing run output →
  [operational-logs.md](operational-logs.md).

**Safety, up front:**

- Files under `.orbit/state/job-runs/` and `.orbit/state/audit/` are evidence.
  Never edit them to "fix" a run.
- Never kill a process before matching run id → `pid`/`pgid`/task id(s)/command;
  terminate the process group for that run id only. Never kill a parent auto or
  gate run without verifying it owns the same task(s).
- Top-level `state: failed` is not a diagnosis — find the first failed step or
  activity.
- Task state and run state are the durable handoff. Never parse agent prose in
  their place.
- If Orbit's own tooling or diagnostics mislead you, record friction
  ([friction.md](friction.md)).
