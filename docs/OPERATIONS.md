# Orbit Operations Runbook

Day-2 operations for a machine running Orbit: what state lives where, how to back it up,
how to debug stuck job runs, recover a corrupted database, read logs and audit events,
check health, and upgrade safely. Commands below were executed against the current binary
(`orbit 0.9.2`); outputs are real excerpts (paths shortened).

This doc is host-agnostic. Host-specific deployment notes (which units are enabled on
which box, ports, sync jobs) belong in your own ops knowledge base — the pattern used for
Orbit's home deployment is an `environments/<host>/` doc per machine in the (private)
polaris knowledge base, kept out of this repo.

Related: [CONFIG.md](CONFIG.md) (config reference) · [RELEASE.md](RELEASE.md) /
[../RELEASING.md](../RELEASING.md) (cutting releases).

---

## 1. State inventory

Two roots. **Workspace state** lives in `<repo>/.orbit/`; **user/machine state** lives in
`~/.orbit/` (override with `--root <dir>`, highest precedence). Path layout is defined in
`crates/orbit-common/src/types/workspace.rs` (`WorkspacePaths`) and
`crates/orbit-core/src/config/persistence.rs` (`PersistenceConfig`).

### Workspace `.orbit/`

| Path | What it is | Authoritative or regenerable |
|---|---|---|
| `config.yaml` | workspace identity (`workspace_id`) | authoritative |
| `config.toml` | optional workspace runtime config (replaces global — see [CONFIG.md](CONFIG.md)) | authoritative |
| `tasks/` | projection of canonical task bundles: symlinks → `~/.orbit/tasks/workspaces/<ws-id>/` | regenerable (`orbit task reindex`) |
| `adrs/`, `learnings/`, `knowledge/` | canonical ADR / learning / knowledge bundles (files) | **authoritative** |
| `frictions/` | friction records + `tags.yaml` taxonomy | **authoritative** |
| `resources/` | workspace overrides for activities/jobs/executors/policies | authoritative |
| `graph/<branch>.<ver>.db` | code-graph SQLite index, per branch/worktree | regenerable (`orbit graph sync`) |
| `state/layout.version` | plain-text workspace layout version marker | regenerable marker (see §8) |
| `state/layout.lock` | advisory lock taken during layout upgrades | transient |
| `state/semantic.db` | semantic/vector index (docs, learnings, tasks) | regenerable (`orbit semantic index`) |
| `state/scoreboard/` | rolling counters (`pr.json`, `task_review.json`, `duel.json`, …) | mostly regenerable; `duel.json` is an append-only record |
| `state/job-runs/` | legacy file-based run bundles (current runs live in SQLite, §4) | regenerable |
| `state/audit/`, `state/logs/`, `state/diagnostics/`, `state/worktrees/` | scratch dirs; canonical audit + logs are global (§5, §6) | regenerable |

### Global `~/.orbit/`

| Path | What it is | Authoritative or regenerable |
|---|---|---|
| `config.toml` | global runtime config (created by `orbit init`) | authoritative |
| `workspaces.json` | registry of workspaces on this machine | authoritative |
| `orbit.db` (+ `-wal`, `-shm`) | **the** store DB: audit events (`audit_events`, `v2_audit_events`), job runs + checkpoints (`job_runs`, `job_run_steps`), task reservations, ADR/learning indexes, `schema_meta` migration ledger | **authoritative** (history is not derivable) |
| `tasks/index.sqlite` | global task-ID allocator + registry index | regenerable (`orbit task reindex`) |
| `tasks/workspaces/<ws-id>/<task-id>/` | canonical task bundles (survive repo moves) | **authoritative** |
| `resources/`, `skills/` | default activity/job/executor/policy defs, skills | regenerable (`orbit init` reseeds) |
| `state/logs/orbit.jsonl` (+ rotated archives) | unified JSONL log sink for all orbit processes | disposable |
| `embed/` | semantic-search companion binary + models | regenerable (`orbit semantic install`) |
| `bin/` | installed orbit binary (when installed via `install.sh`) | reinstallable |

### Git-committed vs local

`orbit workspace init` appends a single `.orbit` line to the repo's `.gitignore` — by
default the whole directory stays local. Repos that want project memory (ADRs, learnings)
in git use a selective pattern instead, keeping DBs, locks, and runtime state out:

```gitignore
.orbit/*
!.orbit/config.yaml
!.orbit/adrs/
!.orbit/learnings/
!.orbit/knowledge/
!.orbit/frictions/
!.orbit/resources/
# never commit runtime state
.orbit/**/*.sqlite
.orbit/**/*.sqlite-*
.orbit/**/*.db
.orbit/**/*.db-*
.orbit/**/*.lock
.orbit/state/
```

---

## 2. Backup and restore

### What to back up

- **Workspace**: the `.orbit/` directory (cheap; skip `state/` and `graph/` if you want —
  both regenerate). If the repo commits ADRs/learnings via the selective gitignore, git
  already is the backup for those.
- **Global root**: `~/.orbit/config.toml`, `workspaces.json`, `tasks/` (canonical
  bundles), and `orbit.db` — that last one holds the non-derivable audit + run history.
- **Safe to lose / regenerate**: `graph/*.db`, `state/semantic.db`, `tasks/index.sqlite`,
  `~/.orbit/embed/`, `~/.orbit/state/logs/`, scoreboard counters.

### SQLite consistency (WAL)

All Orbit DBs run in WAL mode. A plain `cp` of a live `*.db` without its `-wal`/`-shm`
sidecars can produce a torn copy. Safe options, in order of preference:

```sh
# 1. Cold copy — no orbit processes running (stop orbit-web / timers first):
cp -a ~/.orbit ~/orbit-backup-$(date +%F)

# 2. Live, consistent single-DB snapshot (works while orbit runs):
sqlite3 ~/.orbit/orbit.db "VACUUM INTO '/backups/orbit.db'"
# or: sqlite3 ~/.orbit/orbit.db ".backup /backups/orbit.db"

# 3. Portable task backup / machine migration (tasks only):
orbit task export --all -o tasks-backup.tar.zst
```

Both `VACUUM INTO` and `.backup` produce a checkpointed, sidecar-free file. If you must
file-copy a live DB, copy `*.db`, `*.db-wal`, and `*.db-shm` together.

### Restore

```sh
# stop any local Orbit workers, MCP servers, and dashboards before restoring

# put the file back and drop stale sidecars from the old incarnation
cp /backups/orbit.db ~/.orbit/orbit.db
rm -f ~/.orbit/orbit.db-wal ~/.orbit/orbit.db-shm

# rebuild derived indexes as needed
orbit task reindex
orbit semantic index      # if semantic search is installed
orbit graph sync          # per workspace, on demand

orbit doctor              # verify (§7)
```

Task bundles restored by file copy (e.g. rsync of `~/.orbit/tasks/`) need
`orbit task reindex` afterward; for cross-machine moves prefer
`orbit task export` / `orbit task import --on-conflict=renumber`.

---

## 3. Stuck-job debugging

Two surfaces: `orbit job` manages job *definitions* plus resume/replay;
`orbit run` is the run *observability* surface.

```sh
orbit run history --limit 20        # recent runs (add -j <job_id> to filter)
orbit run show   <run_id>           # per-step state table
orbit run logs   <run_id>           # raw stdout/stderr blobs (agent steps)
orbit run events <run_id>           # audit events for the run
orbit run trace  <run_id>           # parent/child event tree
orbit job resume <run_id>           # resume from step checkpoints
orbit job replay <run_id>           # re-run from step 0
```

### Run states

`pending → running → success | failed | timeout | cancelled | interrupted`
(plus transient `retrying` and step-level `skipped`). **`interrupted`** means the run was
orphaned: its owner process died (crash, SIGKILL, reboot) without finalizing the run. A
job that genuinely failed is `failed`; `interrupted` means the *worker* died.

### Orphan scan

Every run records its owner `pid` + a pid-start-time token; pipeline workers claim their
queued run at startup, so `pending` runs carry an owner too [ORB-10070]. A reconcile pass
probes liveness and finalizes conclusively-orphaned runs to `interrupted` (releasing
their task reservations): `running` runs with a dead owner, `pending` runs whose claimed
worker died, and `pending` runs never claimed within a 30-minute grace window (e.g.
queued children stranded when their parent run was interrupted by a reboot). It runs
best-effort at workspace open and lazily on `orbit run history/show`; `orbit doctor`'s
`job-runs` check reports orphans read-only, and `orbit run cancel <run_id>` terminalizes
a stuck run on demand. A run whose pid is alive but unverifiable is deliberately left
alone — never assume a long-`running` run is stuck without checking the pid yourself.

Real sequence (worker SIGKILLed mid-step):

```
$ orbit run history --limit 1
│ RUN_ID                 JOB_ID            ATTEMPT   STATE         ERROR_MESSAGE                              │
│ jrun-20260704-0927-2   demo_sleep_long   1         interrupted   job run marked interrupted because         │
│                                                                  recorded worker process is no longer alive │
│                                                                  (reason=process_not_found, pid=154953, …)  │
```

### Checkpoints and resume

The v2 executor checkpoints after every completed top-level step into the run row itself
(`job_runs.pipeline_state_json` in `~/.orbit/orbit.db`) — there is no separate checkpoint
file. `orbit job resume <run_id>` accepts runs in `interrupted`, `failed`, or `timeout`
(anything else errors: *"resume requires an interrupted, failed, or timed-out run"*). It
starts a **new linked run** (`attempt + 1`, `retry_source_run_id` set); checkpointed steps
are skipped with their outputs replayed into the pipeline:

```
$ orbit job resume jrun-20260704-0927-2
$ orbit run events jrun-20260704-0928
│ 2026-07-04T09:28:15Z   -         run.started         cli:demo_sleep_long                                   │
│ 2026-07-04T09:28:15Z   nap_one   step.skipped        step=nap_one reason=resume: step already completed    │
│                                                      in checkpointed run (index 0)                         │
│ 2026-07-04T09:28:15Z   nap_two   step.started        nap_two                                               │
│ 2026-07-04T09:28:17Z   run.finished        success                                                         │
```

Resume needs the job present in the catalog (`orbit job list --all`); a run started from
a raw YAML path can only be resumed once that YAML is registered under
`resources/jobs/`. A run with no successful checkpoints degrades to a full replay.

---

## 4. Corrupted-DB recovery

`orbit doctor` runs `PRAGMA quick_check` on the store DB (`~/.orbit/orbit.db`). Two
failure shapes, both exit 1:

```
# page-level corruption, store still opens:
│ database         ERROR     integrity check failed: store error: quick_check: database disk image is malformed │

# severe corruption, store cannot open (doctor aborts before the table):
error: store error: database disk image is malformed
```

Recovery, in order:

1. **Restore from backup** (§2). `orbit.db` is authoritative history — prefer this.
2. **Salvage with sqlite3** (verified round-trip; rows on corrupted pages are lost —
   in testing a corrupted `job_runs` table came back empty while other tables survived):

   ```sh
   sqlite3 ~/.orbit/orbit.db ".recover" | sqlite3 ~/.orbit/orbit.recovered.db
   sqlite3 ~/.orbit/orbit.recovered.db "PRAGMA integrity_check;"   # expect: ok
   mv ~/.orbit/orbit.recovered.db ~/.orbit/orbit.db
   rm -f ~/.orbit/orbit.db-wal ~/.orbit/orbit.db-shm
   orbit doctor
   ```

3. **Regenerate, if the broken DB is derivable**:

   | DB | Derivable? | Rebuild |
   |---|---|---|
   | `<ws>/.orbit/graph/*.db` | yes | `orbit graph clean && orbit graph sync` |
   | `<ws>/.orbit/state/semantic.db` | yes | delete file, `orbit semantic index` |
   | `~/.orbit/tasks/index.sqlite` | yes (from task bundles) | `orbit task reindex` |
   | `~/.orbit/orbit.db` | **no** — audit + run history | restore or salvage; last resort: delete and lose history (task/ADR/learning *content* lives in file bundles and survives) |

---

## 5. Log locations and rotation

All orbit processes (CLI, `orbit web serve`, MCP server) append structured tracing events
to one global JSONL sink:

```
~/.orbit/state/logs/orbit.jsonl        # override: $ORBIT_LOG_PATH
```

One JSON object per line: `{"timestamp", "level", "target", "fields": {..., "message"}}`.
Secret-looking values (env vars matching `TOKEN`/`SECRET`/`PASSWORD`/`API_KEY`,
`Authorization`/`x-api-key` headers, `sk-…` keys) are redacted before they reach the sink.

**Rotation** (size-based, checked once at process start, implemented in
`orbit-common/src/utility/log_rotation.rs`): when the active file exceeds the per-file
cap it is renamed to `orbit.jsonl.<UTC-timestamp>`; archives older than the retention
window are deleted, then oldest-first until the total-size cap holds. Defaults: **100 MB
per file, 500 MB total, 7 days retention**. Override in `~/.orbit/config.toml`:

```toml
[runtime]
log_retention_days = 7
log_max_total_mb = 500
log_max_file_mb = 100
```

**Reading:**

```sh
orbit log tail -n 100                        # four-column view of recent events
orbit log tail -f --level warn               # follow, warnings and up
orbit log tail --target orbit.policy --since 1h
orbit log tail --json                        # raw JSONL lines

# jq directly on the sink:
jq -r 'select(.level=="ERROR")
       | "\(.timestamp) \(.target) \(.fields.message)"' ~/.orbit/state/logs/orbit.jsonl
```

`RUST_LOG` controls the tracing filter for any orbit process
(e.g. `RUST_LOG=debug orbit task list` — standard `EnvFilter` syntax).

**Routine sweep log (macOS).** The launchd agent (`com.orbit.sweep`, installed by
`orbit routine init --install-clock`) redirects `orbit sweep` stdout/stderr to a separate
file, since it is not the JSONL tracing sink:

```
~/.orbit/logs/sweep.log
```

Two things keep it bounded on an always-on host ([ORB-00423]): `orbit sweep` prints only
noteworthy rows by default — fires, retries, baselines, and errors, plus a one-line
heartbeat when a pass had nothing due (`--verbose` restores a row per routine) — and each
pass opportunistically rolls + prunes `sweep.log` through the same
`log_rotation` machinery and `[runtime]` caps as the JSONL sink above (rename-based archives
`sweep.log.<UTC-timestamp>`). On Linux the sweep unit logs to the journal instead, which
rotates on its own.

---

## 6. Audit trail

Audit events are stored in **SQLite, not JSONL** — tables `audit_events` (one row per CLI
/ MCP invocation, written by an RAII guard so even crashes record a failure row) and
`v2_audit_events` (run → step → activity envelope tree, keyed by `run_id`) in
`~/.orbit/orbit.db`. Audit *write* failures are non-fatal: the run continues and is
flagged incomplete rather than crashing.

```sh
orbit audit list --since 1h --status failure     # recent failures
orbit audit list --json --limit 100              # full event objects
orbit audit show <id>
orbit audit stats --since 7d
orbit audit export --output audit.json           # JSON or --format csv
orbit audit prune --older-than 90d
```

Per-invocation event fields (from `orbit audit list --json`; struct
`orbit-common/src/types/audit_event.rs`): `id`, `execution_id`, `timestamp`, `command`,
`subcommand`, `tool_name`, `target_type`, `target_id`, `role`,
`status` (`success|failure|denied`), `exit_code`, `duration_ms`, `working_directory`,
`arguments_json`, `stdout_truncated`, `stderr_truncated`, `error_message`, `host`, `pid`,
`session_id`, `task_id`, `job_run_id`, `activity_id`, `step_index`.

Worked example — what failed in the last day, and why:

```sh
orbit audit export --output /tmp/audit.json
jq -r '.[] | select(.status=="failure")
       | "\(.timestamp[0:19]) \(.command) \(.subcommand // "-") \(.error_message // "-")"' /tmp/audit.json
```

For a single run's audit trail use `orbit run events <run_id>` / `orbit run trace <run_id>` (§3).

---

## 7. Health and self-diagnosis

### `orbit doctor`

Seven checks, in order — every check degrades to a row rather than aborting (unless the
store itself cannot open):

| Check | What it verifies |
|---|---|
| `config` | layered config parses (`~/.orbit/config.toml` + workspace `config.toml`) |
| `database` | store DB `PRAGMA quick_check` + schema-ledger version vs this binary |
| `disk-space` | free space on the volume holding `.orbit` (warn < 1 GiB or < 5%, fail < 256 MiB or < 1%) |
| `semantic-index` | stale embedding rows (skipped if never indexed) |
| `graph-index` | newest `graph/*.db` opens read-only (skipped if never built) |
| `stale-locks` | `.lock` files under `state/`, `tasks/`, `learnings/`, `adrs/.locks/` whose recorded holder pid is dead |
| `job-runs` | orphaned `running` runs whose owner process is gone (§3) |

```
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
│ job-runs         ok        no orphaned running job runs                                     │
0 failure(s), 1 warning(s).
```

**Exit codes**: nonzero **only when at least one check is ERROR**; warnings/skips exit 0 —
safe to wire into cron/CI alerting. `--json` emits `[{"check","status","message"}]` with
lowercase statuses. Lock files flagged by `stale-locks` carry holder diagnostics
(`{pid, acquired_at, label}`) and are safe to delete once the holder pid is dead.

### `/healthz`

Served by the dashboard (`orbit web serve`, loopback-only, default `127.0.0.1:7878`):

```sh
curl -s localhost:7878/healthz                       # -> "ok" (cheap liveness, always 200)
curl -s 'localhost:7878/healthz?detailed=true' | jq  # readiness; HTTP 503 if any check fails
```

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

Each detailed check is time-bounded (2 s) and per-workspace (`sqlite_writable` does a
`BEGIN IMMEDIATE; ROLLBACK` — no mutation). Point uptime monitoring at the detailed form.

### Routine clock

Routines are Orbit's scheduling surface. Install or refresh the host clock with
`orbit routine init --install-clock`; inspect due work and recent fires through
`orbit routine list`, `orbit sweep --json`, and `GET /api/routines`. The
`qa-sweep` auto-task definition is ordinary workspace data under
`.orbit/auto_tasks/`, processed by the generic auto-task scheduler routine.

---

## 8. Upgrades

Two version ledgers guard `.orbit/` state, and both **auto-apply on workspace open**:

- **Workspace layout** — plain-text marker `.orbit/state/layout.version` plus an ordered
  migration registry (`orbit-store/src/layout/`). Missing marker = pre-versioning
  workspace, adopted as v1. Upgraders serialize on `state/layout.lock`.
- **Store schema** — `schema_meta` ledger table inside `orbit.db`
  (`orbit-store/src/sqlite/migration/`); each migration + its ledger row commit in one
  transaction.

```sh
orbit migrate --dry-run    # list pending WITHOUT applying (exit 1 when any pending)
orbit migrate              # open the workspace (auto-applies) and report
orbit migrate --json       # machine-readable
```

```
$ orbit migrate --dry-run          # on a pre-upgrade workspace
│ COMPONENT          CURRENT   SUPPORTED │
│ workspace layout   0         1         │
│ store schema       0         1         │
Pending migrations:
  layout v1 (baseline) — adopt the versioned .orbit/ layout (records the current shape; changes nothing)
  schema v1 (baseline)
error: execution failed: 2 migration(s) pending; run `orbit migrate` to apply
```

`--dry-run` is the only way to *see* pending migrations — any command that opens the
workspace (including plain `orbit migrate`) applies them.

**Downgrade guard**: a workspace or DB written by a newer orbit refuses to open instead
of corrupting state:

```
error: schema migration failed: workspace '….orbit' has .orbit layout version 99, newer
than the newest version this orbit binary supports (1); upgrade orbit to open this workspace
```

The same guard exists for the schema ledger (*"store database schema version N is newer
than the newest version this orbit binary supports"*). Fix: upgrade the binary — never
hand-edit `layout.version` to force an open.

**Before a major upgrade** (the CLI prints this hint too):

```sh
cp -a <workspace>/.orbit <workspace>/.orbit.bak     # workspace state
sqlite3 ~/.orbit/orbit.db "VACUUM INTO '/backups/orbit.db'"   # global store (§2)
```

Then upgrade the binary, run `orbit migrate --dry-run` to review, `orbit migrate` to
apply, and `orbit doctor` to verify. Restart any independently managed dashboard process
after swapping the binary.
