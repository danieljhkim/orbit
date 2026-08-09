---
type: runbook
summary: Locate Orbit state and perform WAL-safe backups, restores, and task migrations.
tags: [operations, backup, restore, state, sqlite]
paths: ["crates/orbit-common/src/types/workspace.rs", "crates/orbit-core/src/config/**", "crates/orbit-store/**"]
related_features: [orbit-core, remote-access]
related_artifacts: [ORB-10014, ORB-10294, ORB-10473, ADR-0291]
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
| `graph/`, `knowledge/graph/` | retired graph state left by older Orbit versions | non-authoritative; remove explicitly with `orbit doctor --remove-graph` |
| `state/layout.version` | plain-text workspace layout version marker | regenerable marker (see [upgrades](./upgrades.md)) |
| `state/layout.lock` | advisory lock taken during layout upgrades | transient |
| `state/semantic.db` | semantic/vector index (docs, learnings, tasks) | regenerable (`orbit semantic index`) |
| `state/scoreboard/` | rolling counters (`pr.json`, `task_review.json`, `tokens.json`, …) | mostly regenerable |
| `state/job-runs/` | legacy file-based run bundles; current runs live in SQLite | regenerable |
| `state/audit/`, `state/logs/`, `state/diagnostics/`, `state/worktrees/` | scratch dirs; canonical audit and logs are global | regenerable |

### Global `~/.orbit/`

| Path | What it is | Authoritative or regenerable |
|---|---|---|
| `config.toml` | global runtime config (created by `orbit init`) | authoritative |
| `workspaces.json` | registry of workspaces on this machine (logical workspaces + local checkouts, including declared owner and `owner`/`replica` role) | authoritative |
| `host.toml` | this machine's stable identity (`machine_id`, `host_id`, `mode`); renamed in place by `orbit host rename` | authoritative |
| `registry-cache.json` | satellite cache of one sanitized hub registry snapshot (hosts, aliases, ownership, sanitized freshness) + local receipt time | regenerable (validation-only; refreshed on poll/register) |
| `orbit.db` (+ `-wal`, `-shm`) | **the** store DB: audit events (`audit_events`, `v2_audit_events`), job runs + checkpoints (`job_runs`, `job_run_steps`), task reservations, ADR/learning indexes, host registry (`hosts`, `host_aliases`, `workspace_ownership`, `host_workspace_presence`, `workspace_execution_profiles`, `hub_registry_metadata`), `schema_meta` migration ledger | **authoritative** (history is not derivable) |
| `tasks/index.sqlite` | global task-ID allocator + registry index | regenerable (`orbit task reindex`) |
| `tasks/workspaces/<ws-id>/<task-id>/` | canonical task bundles (survive repo moves) | **authoritative** |
| `resources/activities/`, `resources/jobs/` | managed defaults plus operator-authored activity/job YAML; hidden manifests retain managed content provenance | mixed: current defaults are regenerable, but untracked YAML and `resources/.retired-managed/` backups are **authoritative until reviewed** |
| other `resources/`, `skills/` | default executor/policy defs and skills | regenerable (`orbit init` reseeds) |
| `state/logs/orbit.jsonl` (+ rotated archives) | unified JSONL log sink for all Orbit processes | disposable |
| `embed/` | semantic-search companion binary + models | regenerable (`orbit semantic install`) |
| `bin/` | installed Orbit binary (when installed via `install.sh`) | reinstallable |

> **Live registry refresh (ORB-10294).** A running `orbit web serve` no longer needs a
> restart to pick up `workspaces.json` changes. It reloads the registry at each request
> boundary, so a native `orbit workspace init` / `remove` — or a re-pointed checkout
> binding — becomes visible through `/api/workspaces` and routable through the
> workspace-scoped API on the next request; a removed workspace's cached runtime is evicted
> without disturbing the others. **Operator recovery semantics:** a checkout path that
> disappears after startup is reported `invalid` (inactive) rather than deleted — restore or
> re-point the path and the next request re-activates it, no restart needed. A malformed or
> half-written `workspaces.json` (e.g. an editor mid-save) never replaces the last good
> in-memory set: the server keeps serving the previous workspaces and logs a credential-safe
> diagnostic (the registry path plus the parse error, never the file contents) until the file
> parses again. A malformed registry present *at server startup* is still fatal — fix the file
> before launching. See [remote-access design §2.1](../design/remote-access/2_design.md) and
> [ADR-0234](../design/remote-access/4_decisions.md).

> **Managed activity/job refresh (ORB-10684 / ADR-0346).** `orbit init` records
> the digest it wrote for each bundled activity and job in the resource
> directory's `.orbit-managed-assets.json`. A later refresh deletes a retired
> file only when it still matches that digest. Locally modified retired files
> move to `resources/.retired-managed/{activities,jobs}/`, outside active
> catalogs; back up and review those files as operator data. On a legacy root
> with no manifest, non-matching YAML stays in place and init names it in a
> warning. Move or delete only the named stale file after confirming it came
> from an older release, then rerun `orbit init` and the affected list command.

### Retired graph state and task selectors

ADR-0291 retired graph as an Orbit capability. Task `symbol:<path>#<symbol>:<kind>` context
selectors now use only `<path>` as a canonical workspace-contained file anchor; the symbol and
kind are opaque descriptive metadata. No health, task, or dashboard path probes graph state or
resolves symbols through it.

Older worktrees may still contain worktree-local `.orbit/graph`, while the shared workspace may
contain `.orbit/knowledge/graph`. `orbit doctor --remove-graph` removes exactly those two
locations and is safe to repeat. Ordinary `orbit doctor` is read-only with respect to both.

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

- **Workspace:** the `.orbit/` directory. You may skip regenerable `state/` and the retired
  `graph/` and `knowledge/graph/` locations. If the repo commits ADRs and learnings through the
  selective gitignore, git already backs those up.
- **Global root:** `~/.orbit/config.toml`, `workspaces.json`, `tasks/` (canonical bundles),
  `orbit.db`, and `resources/` whenever it contains operator-authored YAML or
  `.retired-managed/` recovery copies. The database holds non-derivable audit and run history.
- **Safe to lose or regenerate:** retired `graph/` and `knowledge/graph/`, `state/semantic.db`,
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
orbit doctor --remove-graph # remove retired local/shared graph state, if present

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
