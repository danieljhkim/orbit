---
type: runbook
summary: Review, apply, and verify Orbit workspace-layout and store-schema migrations safely.
tags: [operations, upgrades, migrations, recovery]
paths: ["crates/orbit-store/src/layout/**", "crates/orbit-store/src/sqlite/migration/**"]
related_features: [orbit-core]
related_artifacts: [ORB-10014]
---

# Upgrade Orbit Safely

Use this runbook before replacing an Orbit binary that may introduce workspace-layout or
store-schema migrations.

## Understand the version ledgers

Two ledgers guard `.orbit/` state and auto-apply on workspace open:

- **Workspace layout:** `.orbit/state/layout.version` plus an ordered migration registry in
  `orbit-store/src/layout/`. A missing marker means a pre-versioning workspace and is adopted
  as v1. Upgraders serialize on `state/layout.lock`.
- **Store schema:** the `schema_meta` ledger table inside `orbit.db`, backed by
  `orbit-store/src/sqlite/migration/`. Each migration and its ledger row commit in one
  transaction.

## Back up before a major upgrade

Stop or quiesce independently managed Orbit processes, then create consistent backups:

```sh
cp -a <workspace>/.orbit <workspace>/.orbit.bak
sqlite3 ~/.orbit/orbit.db "VACUUM INTO '/backups/orbit.db'"
```

See [Inventory and protect Orbit state](./state-and-backup.md) for WAL-safe alternatives and
the complete authoritative-state inventory.

## Review and apply migrations

```sh
orbit migrate --dry-run    # list pending without applying; exit 1 when any are pending
orbit migrate              # same safe inspection default
orbit migrate --confirm    # open the workspace, auto-apply, and report
orbit migrate --json       # machine-readable inspection report
```

Example dry run on a pre-upgrade workspace:

```text
$ orbit migrate --dry-run
│ COMPONENT          CURRENT   SUPPORTED │
│ workspace layout   0         1         │
│ store schema       0         1         │
Pending migrations:
  layout v1 (baseline) — adopt the versioned .orbit/ layout (records the current shape; changes nothing)
  schema v1 (baseline)
error: execution failed: 2 migration(s) pending; run `orbit migrate --confirm` to apply
```

Bare `orbit migrate` and the compatibility-explicit `--dry-run` form inspect without opening
the runtime. Applying pending migrations always requires `--confirm`; the command never prompts
or reads stdin.

## Respect the downgrade guard

A workspace or DB written by a newer Orbit refuses to open rather than corrupting state:

```text
error: schema migration failed: workspace '….orbit' has .orbit layout version 99, newer
than the newest version this orbit binary supports (1); upgrade orbit to open this workspace
```

The schema ledger has the same guard:
`store database schema version N is newer than the newest version this orbit binary supports`.
Upgrade the binary. Never hand-edit `layout.version` to force the workspace open.

## Verify the upgrade

After reviewing the dry run:

1. Replace or upgrade the binary.
2. Run `orbit migrate` (or `orbit migrate --dry-run`) to review any still-pending changes.
3. Run `orbit migrate --confirm` to apply them.
4. Run `orbit doctor` and require all relevant checks to pass.
5. Restart any independently managed dashboard process after swapping the binary.

Agent subprocesses inherit an `ORBIT_BIN` pinned to the Orbit executable that dispatched
them, and that executable's directory is placed first on their `PATH`. An operator-set
`ORBIT_BIN` takes precedence. After replacing `~/.orbit/bin/orbit`, restart long-lived Orbit
services and pipeline workers so newly dispatched agents inherit the replacement build. Verify
both the explicit path and the ordinary command resolve the same tool-capable binary:

```sh
~/.orbit/bin/orbit tool run orbit.task.show --input '{"id":"<real-task-id>","model":"codex"}'
command -v orbit
orbit tool run orbit.task.show --input '{"id":"<real-task-id>","model":"codex"}'
```

If `~/.orbit/host.toml` uses a schema newer than an installed binary supports, upgrade and
restart that binary. Never edit `schema_version` downward to bypass the guard.

See [Check Orbit health](./health-checks.md) for `orbit doctor` and dashboard-readiness
semantics. If verification fails, stop writers and restore the backups before further recovery.
