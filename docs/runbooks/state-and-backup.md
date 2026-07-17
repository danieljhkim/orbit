---
type: runbook
summary: Locate Orbit state and perform WAL-safe backups, restores, and task migrations.
tags: [operations, backup, restore, state, sqlite]
paths: ["crates/orbit-common/src/types/workspace.rs", "crates/orbit-core/src/config/**", "crates/orbit-store/**"]
related_features: [orbit-core]
related_artifacts: [ORB-10014]
---

# Inventory and Protect Orbit State

Use this runbook to determine what Orbit state is authoritative, choose what to back up,
restore a store database, or move task bundles between machines.

## State inventory

Two roots hold Orbit state. **Workspace state** lives in `<repo>/.orbit/`;
**user/machine state** lives in `~/.orbit/` (override with `--root <dir>`, highest
precedence). Path layout is defined in
`crates/orbit-common/src/types/workspace.rs` (`WorkspacePaths`) and
`crates/orbit-core/src/config/persistence.rs` (`PersistenceConfig`).

### Workspace `.orbit/`

| Path | What it is | Authoritative or regenerable |
|---|---|---|
| `config.yaml` | workspace identity (`workspace_id`) | authoritative |
| `config.toml` | optional workspace runtime config (replaces global—see [CONFIG.md](../CONFIG.md)) | authoritative |
| `tasks/` | projection of canonical task bundles: symlinks → `~/.orbit/tasks/workspaces/<ws-id>/` | regenerable (`orbit task reindex`) |
| `adrs/`, `learnings/`, `knowledge/` | canonical ADR / learning / knowledge bundles (files) | **authoritative** |
| `frictions/` | friction records + `tags.yaml` taxonomy | **authoritative** |
| `resources/` | workspace overrides for activities/jobs/executors/policies | authoritative |
| `graph/<branch>.<ver>.db` | code-graph SQLite index, per branch/worktree | regenerable (`orbit graph sync`) |
| `state/layout.version` | plain-text workspace layout version marker | regenerable marker (see [upgrades](./upgrades.md)) |
| `state/layout.lock` | advisory lock taken during layout upgrades | transient |
| `state/semantic.db` | semantic/vector index (docs, learnings, tasks) | regenerable (`orbit semantic index`) |
| `state/scoreboard/` | rolling counters (`pr.json`, `task_review.json`, `duel.json`, …) | mostly regenerable; `duel.json` is an append-only record |
| `state/job-runs/` | legacy file-based run bundles; current runs live in SQLite | regenerable |
| `state/audit/`, `state/logs/`, `state/diagnostics/`, `state/worktrees/` | scratch dirs; canonical audit and logs are global | regenerable |

### Global `~/.orbit/`

| Path | What it is | Authoritative or regenerable |
|---|---|---|
| `config.toml` | global runtime config (created by `orbit init`) | authoritative |
| `workspaces.json` | registry of workspaces on this machine | authoritative |
| `orbit.db` (+ `-wal`, `-shm`) | **the** store DB: audit events (`audit_events`, `v2_audit_events`), job runs + checkpoints (`job_runs`, `job_run_steps`), task reservations, ADR/learning indexes, `schema_meta` migration ledger | **authoritative** (history is not derivable) |
| `tasks/index.sqlite` | global task-ID allocator + registry index | regenerable (`orbit task reindex`) |
| `tasks/workspaces/<ws-id>/<task-id>/` | canonical task bundles (survive repo moves) | **authoritative** |
| `resources/`, `skills/` | default activity/job/executor/policy defs, skills | regenerable (`orbit init` reseeds) |
| `state/logs/orbit.jsonl` (+ rotated archives) | unified JSONL log sink for all Orbit processes | disposable |
| `embed/` | semantic-search companion binary + models | regenerable (`orbit semantic install`) |
| `bin/` | installed Orbit binary (when installed via `install.sh`) | reinstallable |

### Git-committed versus local state

`orbit workspace init` appends a single `.orbit` line to the repo's `.gitignore`; by
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

## Back up Orbit

### What to back up

- **Workspace:** the `.orbit/` directory. You may skip `state/` and `graph/` because both
  regenerate. If the repo commits ADRs and learnings through the selective gitignore, git
  already backs those up.
- **Global root:** `~/.orbit/config.toml`, `workspaces.json`, `tasks/` (canonical bundles),
  and `orbit.db`. The database holds non-derivable audit and run history.
- **Safe to lose or regenerate:** `graph/*.db`, `state/semantic.db`,
  `tasks/index.sqlite`, `~/.orbit/embed/`, `~/.orbit/state/logs/`, scoreboard counters.

### Preserve SQLite consistency

All Orbit DBs run in WAL mode. A plain `cp` of a live `*.db` without its `-wal` and
`-shm` sidecars can produce a torn copy. Use one of these options, in order of preference:

```sh
# 1. Cold copy—no Orbit processes running (stop orbit-web and timers first):
cp -a ~/.orbit ~/orbit-backup-$(date +%F)

# 2. Live, consistent single-DB snapshot (works while Orbit runs):
sqlite3 ~/.orbit/orbit.db "VACUUM INTO '/backups/orbit.db'"
# or: sqlite3 ~/.orbit/orbit.db ".backup /backups/orbit.db"

# 3. Portable task backup or machine migration (tasks only):
orbit task export --all -o tasks-backup.tar.zst
```

Both `VACUUM INTO` and `.backup` produce a checkpointed, sidecar-free file. If you must
file-copy a live DB, copy `*.db`, `*.db-wal`, and `*.db-shm` together.

## Restore Orbit

Stop local Orbit workers, MCP servers, and dashboards before restoring. The commands below
overwrite the current store database; retain a copy until verification succeeds.

```sh
# Put the snapshot back and drop stale sidecars from the old incarnation.
cp /backups/orbit.db ~/.orbit/orbit.db
rm -f ~/.orbit/orbit.db-wal ~/.orbit/orbit.db-shm

# Rebuild derived indexes as needed.
orbit task reindex
orbit semantic index      # if semantic search is installed
orbit graph sync          # per workspace, on demand

orbit doctor              # verify; see health-checks.md
```

Task bundles restored by file copy (for example, an rsync of `~/.orbit/tasks/`) need
`orbit task reindex` afterward. For cross-machine moves, prefer
`orbit task export` / `orbit task import --on-conflict=renumber`.

## Verification

Run `orbit doctor` and confirm that the `database` row reports `ok`; see
[Check Orbit health](./health-checks.md) for exit-code semantics. Then show or search a
known task to confirm that task bundles were reindexed.

Related: [Recover a corrupted database](./database-recovery.md) ·
[Upgrade Orbit safely](./upgrades.md).
