---
title: Routines — Decisions
owner: claude
last_updated: 2026-07-03
status: Draft
feature: routines
doc_role: decisions
type: design
summary: ADR log for the routines scheduler; candidate decisions pending global ID allocation.
tags: [routines, scheduler]
paths: ["crates/orbit-core/src/routines/**"]
related_features: [routines, activity-job]
related_artifacts: [ORB-10001]
---

# Routines — Decisions

This is the append-only ADR log for the `routines` feature. Entries are ordered by
ascending global ADR number, each keyed on an ID allocated through `orbit.adr.add`; the
ADR store is the source of truth for status, owner, `related_features`, and `related_tasks`.

> **No ADRs allocated yet.** The feature is a proposal; per convention, global IDs are
> allocated via `orbit.adr.add` before any `## ADR-` heading is written, so this log starts
> with the candidate decisions below. Each candidate meets the three-part ADR bar (real
> alternative, forward constraint, non-trivial cost) and should be allocated as `Proposed`
> when the feature's first task is cut, then written up here under its allocated heading.

---

## Candidate decisions (pending allocation)

### Candidate: definitions are git-shared; scheduler state is host-local and never synced

- **Alternative considered:** syncing scheduler state (last fires, pauses) between hosts so
  either machine can answer for both.
- **Decision sketch:** routine YAML is versioned and converges via git; fires, pauses, and
  locks live in a host-local SQLite `routine_store`. No scheduler network protocol exists.
- **Cost:** cross-host observability requires asking each host (no single pane of glass),
  and a definition edit is only as fresh on the other host as its last `git pull`.

### Candidate: discovery via the workspace registry + `[routines] role = "source"`, not a host-level pointer file

- **Alternative considered:** a `~/.orbit/host.toml` entry pointing at one designated
  control-workspace path per host (explicit two-way handshake).
- **Decision sketch:** sweep enumerates `~/.orbit/workspaces.json` and collects routines
  from registered workspaces whose versioned config opts in — the `ship-sweep` /
  `auto_ship` precedent. Centralizing all routines in polaris is constellation convention,
  not Orbit mechanism. `host.toml` survives only to carry `host_id`.
- **Cost:** any registered workspace's config can make it a routine source — the review
  boundary widens from one blessed repo to every registered workspace's `config.toml`, and
  sweep behavior now depends on registry hygiene (stale registered paths must be skipped
  loudly, not silently).

### Candidate: hosts are pinned explicitly; no cross-host coordination in v1

- **Alternative considered:** a "run on exactly one host" mode backed by leases.
- **Decision sketch:** `hosts:` lists every host a routine fires on; listing two hosts
  means two independent fires. Failover is out of scope until a real routine needs it.
- **Cost:** no routine survives its pinned host being down; adding leases later introduces
  a second, coordinated mode whose semantics diverge from everything shipped in v1.

### Candidate: targets are catalog references only — no inline command payloads

- **Alternative considered:** a `run: {type: shell, command: ...}` payload for small
  chores, which was the original design sketch before [ADR-0194] surfaced.
- **Decision sketch:** `target:` accepts `job:<name>` / `activity:<name>` resolved through
  the existing catalog, load-time-validated, executing under existing activity/job policy.
  Shell-like chores become `deterministic` activities or jobs in the source workspace.
- **Cost:** every new chore requires authoring a catalog asset (higher friction than a
  one-line command), and scheduler capability is permanently coupled to catalog capability.

### Candidate: the OS owns the clock — stateless `orbit sweep` under launchd/systemd, no daemon

- **Alternative considered:** a resident `orbit schedulerd` owning timers in-process.
- **Decision sketch:** launchd (`StartInterval`) and a systemd timer (`Persistent=true`)
  invoke `orbit sweep` every minute; sweep is stateless-in, durable-out. Missed-fire
  semantics are split between the OS layer (wake/persistence) and per-routine
  `missed_run` policy.
- **Cost:** minute granularity is a hard floor, event triggers are structurally impossible
  in v1, and correct behavior depends on two platform-specific unit files that must be
  kept in parity and tested on both platforms.

---

## Task References

- [ORB-10001] — authored this design-doc folder (proposal; no implementation).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
