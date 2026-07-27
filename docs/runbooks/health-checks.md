---
type: runbook
summary: Check Orbit workspace, database, dashboard, log-sink, job-run, and routine-clock health.
tags: [operations, health, doctor, dashboard, routines]
paths: ["crates/orbit-cmd/src/doctor.rs", "crates/orbit-core/src/command/job/run/reconcile.rs"]
related_features: [orbit-core, activity-job, routines]
related_artifacts: [ORB-10005, ORB-10070, ORB-10473, ORB-10501, ADR-0291, ADR-0296]
---

# Check Orbit Health

Use this runbook for local diagnosis, readiness monitoring, or verification after a restore,
database recovery, or upgrade.

## Run `orbit doctor`

`orbit doctor` performs eight checks in order. Every check degrades to a row rather than
aborting unless the store itself cannot open.

| Check | What it verifies |
|---|---|
| `config` | layered config parses (`~/.orbit/config.toml` + workspace `config.toml`) |
| `database` | store DB `PRAGMA quick_check` + schema-ledger version versus this binary |
| `disk-space` | free space on the volume holding `.orbit` (warn below 1 GiB or 5%; fail below 256 MiB or 1%) |
| `semantic-index` | stale embedding rows; skipped if never indexed |
| `stale-locks` | `.lock` files under `state/`, `tasks/`, `learnings/`, and `adrs/.locks/` whose recorded holder PID is dead |
| `job-runs` | orphaned `pending` or `running` runs whose owner process is gone |
| `task-relations` | unresolved relation/dependency targets that would block a task-index rebuild |
| `id-allocations` | learning/ADR ids pinned to a worktree that no longer exists, with no readable body |

Example:

```text
$ orbit doctor
│ CHECK            STATUS    DETAILS                                                          │
│ config           ok        valid (~/.orbit/config.toml)                                     │
│ database         ok        quick_check ok; schema version 1 matches this binary             │
│ disk-space       ok        11.2 GiB free of 65.6 GiB (17.1%) on the volume holding …/.orbit │
│ semantic-index   skipped   no semantic embeddings indexed yet                               │
│ stale-locks      warning   1 lock file(s) left by dead holders (the OS already released     │
│                            the flock; safe to delete): …/state/layout.lock                  │
│                            (dead pid 154488, op: layout upgrade, since 2026-07-04T09:25…)   │
│ job-runs         ok        no orphaned job runs                                              │
│ task-relations   ok        no unresolved relation/dependency targets                        │
│ id-allocations   ok        no learning/ADR allocations pinned to a missing worktree         │
0 failure(s), 1 warning(s).
```

The command exits nonzero only when at least one check is `ERROR`; warnings and skips exit
zero. `--json` emits an array of objects with `check`, `status`, and `message` fields; statuses
are lowercase. Lock files flagged by `stale-locks` include holder diagnostics and are safe to
delete only after confirming the holder PID is dead.

An `id-allocations` warning means an id was allocated inside a worktree that has since been
reaped, before its body was merged: the body is unrecoverable and the row would otherwise stay
in `orbit learning list --include-remote` (and the ADR list) forever. Confirm the named
worktrees really are gone — a volume that is merely unmounted reads the same way — then retire
the rows with `orbit doctor --fix-orphaned-allocations`. The repair re-verifies every row
before writing, refuses any that became readable again, and flips the row to `abandoned`
instead of deleting it, so the retired ids are never reissued (ADR-0296, ORB-10501). Without
the flag, `orbit doctor` only reports.

Graph is retired under ADR-0291 and is not inspected by ordinary health checks. To remove
leftover state explicitly, run `orbit doctor --remove-graph`. This deletes only the current
worktree's `.orbit/graph` and the shared workspace's `.orbit/knowledge/graph`; it is
idempotent when either is absent. Combine it with `--json` for a single JSON result with no
cleanup prose on stdout. Without `--remove-graph`, `orbit doctor` leaves both locations
untouched.

## Probe dashboard health

The loopback-only dashboard (`orbit web serve`, default `127.0.0.1:7878`) exposes liveness
and readiness:

```sh
curl -s localhost:7878/healthz                       # -> "ok"; cheap liveness, always 200
curl -s 'localhost:7878/healthz?detailed=true' | jq  # readiness; HTTP 503 if any check fails
```

Example detailed response:

```json
{
  "status": "ok",
  "workspaces_open": 1,
  "checks": [
    {"name": "sqlite_writable", "status": "ok",   "detail": "store database accepts writes", "workspace": "default"},
    {"name": "log_sink",        "status": "ok",   "detail": "~/.orbit/state/logs/orbit.jsonl accepts appends"}
  ]
}
```

Each detailed check is time-bounded to two seconds and runs per workspace.
`sqlite_writable` executes `BEGIN IMMEDIATE; ROLLBACK` without mutation. Point uptime
monitoring at the detailed form.

## Check the routine clock

Routines are Orbit's scheduling surface. Install or refresh the host clock with
`orbit routine init --install-clock`. Inspect due work and recent fires through:

```sh
orbit routine list
orbit sweep --json
```

The dashboard also exposes `GET /api/routines`. Auto-task definitions such as
`qa-sweep` and `artifact-deprecation-review` are ordinary workspace data under
`.orbit/auto_tasks/`, processed by the generic auto-task scheduler routine.
`artifact-deprecation-review` is report-only — it mints a task that lists stale
learning candidates and stale artifact-id comment references via
`execution_summary` and never mutates learnings, ADRs, tasks, friction
records, or comments (ORB-10318, ORB-10348).

## Verification and escalation

Record the exact failing check, full details, binary version, config path, workspace, and
relevant environment overrides before diagnosing. Use
[Recover a corrupted database](./database-recovery.md) for integrity failures and
[Recover stuck job runs](./stuck-job-runs.md) for orphaned-run findings.
