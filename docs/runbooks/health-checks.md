---
type: runbook
summary: Check Orbit workspace, database, dashboard, log-sink, job-run, and routine-clock health.
tags: [operations, health, doctor, dashboard, routines]
paths: ["crates/orbit-cmd/src/doctor.rs", "crates/orbit-core/src/command/job/run/reconcile.rs"]
related_features: [orbit-core, activity-job, routines]
related_artifacts: [ORB-10005, ORB-10070]
---

# Check Orbit Health

Use this runbook for local diagnosis, readiness monitoring, or verification after a restore,
database recovery, or upgrade.

## Run `orbit doctor`

`orbit doctor` performs seven checks in order. Every check degrades to a row rather than
aborting unless the store itself cannot open.

| Check | What it verifies |
|---|---|
| `config` | layered config parses (`~/.orbit/config.toml` + workspace `config.toml`) |
| `database` | store DB `PRAGMA quick_check` + schema-ledger version versus this binary |
| `disk-space` | free space on the volume holding `.orbit` (warn below 1 GiB or 5%; fail below 256 MiB or 1%) |
| `semantic-index` | stale embedding rows; skipped if never indexed |
| `graph-index` | newest `graph/*.db` opens read-only; skipped if never built |
| `stale-locks` | `.lock` files under `state/`, `tasks/`, `learnings/`, and `adrs/.locks/` whose recorded holder PID is dead |
| `job-runs` | orphaned `pending` or `running` runs whose owner process is gone |

Example:

```text
$ orbit doctor
│ CHECK            STATUS    DETAILS                                                          │
│ config           ok        valid (~/.orbit/config.toml)                                     │
│ database         ok        quick_check ok; schema version 1 matches this binary             │
│ disk-space       ok        11.2 GiB free of 65.6 GiB (17.1%) on the volume holding …/.orbit │
│ semantic-index   skipped   no semantic embeddings indexed yet                               │
│ graph-index      skipped   no graph index built (run `orbit graph sync` to create one)      │
│ stale-locks      warning   1 lock file(s) left by dead holders (the OS already released     │
│                            the flock; safe to delete): …/state/layout.lock                  │
│                            (dead pid 154488, op: layout upgrade, since 2026-07-04T09:25…)   │
│ job-runs         ok        no orphaned job runs                                              │
0 failure(s), 1 warning(s).
```

The command exits nonzero only when at least one check is `ERROR`; warnings and skips exit
zero. `--json` emits an array of objects with `check`, `status`, and `message` fields; statuses
are lowercase. Lock files flagged by `stale-locks` include holder diagnostics and are safe to
delete only after confirming the holder PID is dead.

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
    {"name": "graph_index",     "status": "skip", "detail": "no graph index built",           "workspace": "default"},
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
`qa-sweep` and `learning-deprecation-review` are ordinary workspace data under
`.orbit/auto_tasks/`, processed by the generic auto-task scheduler routine.
`learning-deprecation-review` is report-only — it mints a task that lists stale
learning candidates via `execution_summary` and never mutates learnings
(ORB-10318).

## Verification and escalation

Record the exact failing check, full details, binary version, config path, workspace, and
relevant environment overrides before diagnosing. Use
[Recover a corrupted database](./database-recovery.md) for integrity failures and
[Recover stuck job runs](./stuck-job-runs.md) for orphaned-run findings.
