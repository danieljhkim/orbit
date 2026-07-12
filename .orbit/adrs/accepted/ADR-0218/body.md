## Context

Every periodic need in orbit was previously bespoke code: qa-sweep (ORB-10039) was an inline check sweep, ship-sweep had its own systemd unit, and each future recurring chore meant another hardcoded routine. This does not scale — the marginal cost of a new periodic chore is a code change, a review, and a release.

## Decision

Introduce **auto-tasks** as a primitive: a dynamically-defined recurring task template.

- **Record.** An auto-task definition is a git-versioned YAML file under a workspace `.orbit/auto_tasks/<name>.yaml`, discovered by directory scan and parsed fail-closed. We model it on the existing file-backed **routine** record convention (name = identity, `deny_unknown_fields`) rather than the SQLite-indexed learning/ADR convention — no index, id allocator, backend trait, or factory is needed, so the primitive stays small.
- **Schedule.** `schedule` is either a 5-field `cron` or an `every_minutes` interval. Catch-up always collapses: a downtime gap mints one make-up task, not one per missed slot. Cron reuses `routines::due` under `CatchUpOnce`; interval is native.
- **Cursor.** Per-definition last-fired state lives host-local at `<orbit_dir>/state/auto-tasks.json` (workspace-local, gitignored runtime state, matching the scoreboard precedent and L-0041), so the scheduler never churns the git-versioned definition.
- **Scheduler.** One generic deterministic activity `run_auto_task_scheduler`, wrapped in the `auto_task_scheduler_pipeline` job, fired by a seeded `auto_task_scheduler` routine — the only scheduler surface, so a new chore is a new definition, never new code. Because it is a routine, its fires are observable on the dashboard routines surface for free.
- **Dedupe & provenance.** Every minted task carries an `auto-task:<name>` tag; `skip_if_open` uses that tag to avoid firing while a prior instance is still open.
- **Provider-neutral (ADR-0217).** The template carries crew / priority / type only — no turn-based budget knobs anywhere in the schema.
- **CRUD.** Full add/list/show/update/toggle via both CLI (`orbit auto-task …`) and MCP (`orbit.auto_task.*`); disabling is a toggle, not a delete.

## Consequences

- Periodic work becomes data. qa-sweep V1 (ORB-10148) becomes just the first definition; no orbit code change adds a chore.
- The scheduler routine fires minutely, but each definition governs its own cadence via its cursor and catch-up collapse, so a minutely sweep stays cheap and idempotent.
- Definitions are workspace-scoped: the scheduler processes the definitions of the workspace whose routine fired it, not a cross-workspace sweep.
- Cost: a second file-backed record convention (routines, now auto-tasks) exists alongside the SQLite-indexed one; the two conventions must be kept mentally distinct, and auto-task definitions are not full-text searchable via the learning/ADR index.