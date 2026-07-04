---
title: Routines — Overview
owner: claude
last_updated: 2026-07-04
status: Accepted
feature: routines
doc_role: overview
type: design
summary: Durable, git-versioned scheduler primitive that fires catalog jobs/activities on cron triggers, per host, with local state.
tags: [routines, scheduler]
paths: ["crates/orbit-cli/src/command/routine/**", "crates/orbit-core/src/routines/**"]
related_features: [routines, activity-job]
related_artifacts: [ORB-10001, ORB-10021]
---

# Routines — Overview

Routines make Orbit the constellation's single scheduler. A **routine** is a durable,
git-versioned definition of recurring work — a cron trigger, a target from the existing
activity/job catalog, host pinning, and a retry/overlap policy. A stateless **`orbit sweep`**
pass, invoked every minute by the OS scheduler (launchd on macOS, a systemd timer on Linux),
fires whatever is due on the current host through the existing v2 run machinery. Definitions
are shared across hosts via git; all scheduler state (last fires, pauses, locks, run history)
is host-local and never synced. [2_design.md](./2_design.md) is the v1 contract;
[3_vision.md](./3_vision.md) holds what is deliberately out of scope for v1.

> **Status.** v1 shipped in [ORB-10021]; the At a Glance table lists the actual home of
> each concern. Targets are `job:<name>` in v1 — see [ADR-0206] for why `activity:` is
> reserved.

---

## 1. Motivation

Recurring work across the constellation currently has no home. Nothing is scheduled at the
OS level on either host (no crontab, no custom launchd agents); recurring chores — vault
auto-commits, session-log extraction, semantic reindexing — run only when a human or agent
remembers to run them. The work spans two machines (`dk-mac`, `dk-server-1`) with different
availability profiles (a laptop that sleeps vs. an always-on box), so any solution must
handle host pinning, missed-fire policy, and per-host toggles.

Orbit is the right owner because the hard parts already exist here:

1. **Execution.** The [activity-job](../activity-job/1_overview.md) layer provides typed,
   auditable runnable units and an orchestration grammar. Routines add only a *trigger source*
   in front of it — not a new runtime.
2. **Cross-workspace dispatch precedent.** `orbit run ship-sweep` already enumerates the
   global workspace registry from an unattended scheduler and dispatches runs with
   per-workspace opt-in and failure isolation. Routines generalize that shape.
3. **Local durable state.** Orbit already persists run state in workspace-local SQLite
   stores; routine state follows the same pattern.

A scheduler outside Orbit would have to reimplement run history, audit, and policy — the
fragmentation this feature exists to end.

---

## 2. Core Concepts

- **Routine** — a versioned YAML definition in a routine-source workspace: name, trigger,
  target, `hosts`, `enabled`, and policy. The durable unit of scheduling.
- **Target** — what fires: a reference into the existing catalog. v1 dispatches
  `job:<name>`; `activity:<name>` is reserved (wrap the activity in a one-step job — see
  [ADR-0206]). Routines carry no inline commands; the `shell` activity variant was
  removed fail-closed in [ORB-00374] (see [ADR-0194]), and routines inherit that posture.
- **Sweep** — `orbit sweep`, the stateless due-check pass the OS clock invokes every minute.
  Loads definitions, filters for this host, fires due routines, records state, exits.
- **Routine source** — a registered workspace whose config opts in with
  `[routines] role = "source"`. The constellation convention is a single source (polaris),
  but the mechanism permits several.
- **Host identity** — a `host_id` (e.g. `dk-mac`) in host-local config under `~/.orbit/`,
  matched against each routine's `hosts` list.
- **Local pause** — a host-local, SQLite-persisted toggle (`orbit routine pause <name>`)
  that suppresses a routine on one host without touching the shared definition.
- **Fire** — one scheduled dispatch of a routine's target, executed as a normal run with
  `origin: routine/<name>` provenance.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Routine definition type + fail-closed YAML parse | `crates/orbit-common/src/types/routine.rs` | [ORB-10021] |
| Discovery, due computation, dispatch, status | `crates/orbit-core/src/routines/` | [ORB-10021] |
| Host-local scheduler state (fires, pauses) | `crates/orbit-store/src/sqlite/routine_store/` | [ORB-10021] |
| Sweep advisory lock (flock, host-global) | `crates/orbit-store/src/sqlite/routine_store/mod.rs` | [ORB-10021] |
| `orbit sweep` CLI entrypoint | `crates/orbit-cli/src/command/sweep.rs` | [ORB-10021] |
| `orbit routine` CLI (`list/show/pause/resume/init`) | `crates/orbit-cli/src/command/routine/` | [ORB-10021] |
| launchd/systemd unit templates + installer | `crates/orbit-core/assets/clock/` + `src/routines/clock.rs` | [ORB-10021] |
| `[routines] role = "source"` config key | `crates/orbit-core/src/config/{raw,runtime}.rs` | [ORB-10021] |

---

## Task References

- [ORB-10001] — authored this design-doc folder (proposal; no implementation).
- [ORB-10021] — implemented routines v1 (types, store, sweep, CLI, clock units).
- [ORB-00374] — removed the `shell` activity variant and `run_shell` dispatch (fail-closed);
  routines inherit this constraint.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
