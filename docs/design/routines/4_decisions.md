---
title: Routines — Decisions
owner: claude
last_updated: 2026-07-15
status: Accepted
feature: routines
doc_role: decisions
type: design
summary: ADR log for the routines scheduler, including default seeding and workspace-local shipment.
tags: [routines, scheduler]
paths: ["crates/orbit-core/src/routines/**"]
related_features: [routines, activity-job]
related_artifacts: [ORB-10001, ORB-10021, ORB-10207, ADR-0223]
---

# Routines — Decisions

This is the append-only ADR log for the `routines` feature. Entries are ordered by
ascending global ADR number, each keyed on an ID allocated through `orbit.adr.add`; the
ADR store is the source of truth for status, owner, `related_features`, and `related_tasks`.

The five candidate decisions recorded by [ORB-10001] were allocated as ADR-0204..ADR-0208
when the v1 implementation task ([ORB-10021]) was cut, and accepted when it shipped.

---

## ADR-0204 — The OS owns the clock: stateless `orbit sweep` under launchd/systemd, no resident daemon

**Status:** Accepted · 2026-07 · [ORB-10021]

### Context

Something must wake the scheduler. The alternatives were a resident `orbit schedulerd`
owning timers in-process (sub-minute precision, event triggers, but a daemon to supervise
on two platforms), or delegating wake-ups to the OS schedulers that already exist —
launchd on macOS, systemd timers on Linux — invoking a stateless pass every minute.

### Decision

launchd (`StartInterval` 60s) and a systemd user timer (`OnCalendar=*:*:00`,
`Persistent=true`) invoke `orbit sweep` every minute; sweep is stateless-in, durable-out.
Missed-fire semantics split between the OS layer (wake/persistence behavior) and
per-routine `missed_run` policy. There is no resident Orbit daemon. Unit templates live
in `crates/orbit-core/assets/clock/` and are installed by
`orbit routine init --install-clock`.

### Consequences

- No process supervision, crash recovery, or memory-leak surface; a wedged pass affects
  one minute, not the scheduler.
- launchd wake behavior and `Persistent=true` pair with `missed_run: catch_up_once` to
  cover laptop sleep and host downtime.
- Cost: minute granularity is a hard floor and event triggers are structurally impossible
  in v1; correct behavior depends on two platform-specific unit files that must be kept in
  parity and tested on both platforms.

---

## ADR-0205 — Routine discovery via the workspace registry and a versioned `[routines] role = "source"` config key

**Status:** Accepted · 2026-07 · [ORB-10021]

### Context

Sweep must find routine definitions without a resident daemon and without bootstrapping
from the caller's cwd. The alternative was a host-level pointer file in `~/.orbit/host.toml`
naming one designated control workspace per host (an explicit two-way handshake), versus
reusing the global workspace registry the way `orbit run ship-sweep` / `auto_ship` already
does for unattended cross-workspace dispatch.

### Decision

Sweep enumerates `~/.orbit/workspaces.json` and collects `.orbit/routines/*.yaml` from
every registered, active workspace whose versioned `.orbit/config.toml` declares
`[routines] role = "source"`. Centralizing all routines in polaris is constellation
convention, not Orbit mechanism. `~/.orbit/host.toml` survives only to carry `host_id`.

### Consequences

- Setup is what already exists: register the workspace plus one versioned config key;
  both hosts converge through git with no per-host pointer files.
- `orbit routine list` names each routine's source workspace, so provenance stays
  unambiguous with multiple sources.
- Cost: any registered workspace's config can make it a routine source — the review
  boundary widens from one blessed repo to every registered workspace's `config.toml`,
  and sweep correctness now depends on registry hygiene (stale registered paths must be
  skipped loudly, not silently).

---

## ADR-0206 — Routine targets are catalog references only — no inline command payloads

**Status:** Accepted · 2026-07 · [ORB-10021]

### Context

The original sketch allowed a `run: {type: shell, command: ...}` payload for small
chores. [ADR-0194] removed the `shell` activity variant and `run_shell` dispatch
fail-closed; reintroducing arbitrary-command payloads through the scheduler would reopen
that surface on a timer, unattended.

### Decision

`target:` accepts only catalog references resolved at load time; unresolvable targets are
load-time errors. v1 dispatches `job:<name>` — run dispatch is job-shaped
(`submit_pipeline_run` resolves jobs by name; there is no standalone activity run
entrypoint), so `activity:<name>` is reserved and rejected at parse time with guidance to
wrap the activity in a one-step job in the same source workspace. Shell-like chores become
`deterministic` activities or jobs in the source workspace.

### Consequences

- Scheduled execution inherits existing activity/job policy, audit envelopes, and the
  fail-closed posture of [ADR-0194]; the scheduler adds a trigger source, not a new
  execution surface.
- Load-time validation makes a broken reference visible on the next sweep instead of at
  fire time.
- Cost: every new chore requires authoring a catalog asset (higher friction than a
  one-line command), and scheduler capability is permanently coupled to catalog
  capability — including the job-shaped dispatch constraint that keeps `activity:`
  targets out of v1.

---

## ADR-0207 — Routines pin hosts explicitly; no cross-host coordination in v1

**Status:** Accepted · 2026-07 · [ORB-10021]

### Context

Some recurring work should run on exactly one machine. A "run on exactly one of N hosts"
mode needs a lease protocol between hosts that only expose SSH to each other; the
alternative is explicit pinning, where the definition names every host it fires on.

### Decision

Each routine carries a `hosts:` list matched against the host-local `host_id`; there is
no "any host" value in v1. Listing two hosts means two independent fires. Failover stays
out of scope until a real routine needs it.

### Consequences

- Due computation stays purely host-local: no lease table, no network dependency, no
  split-brain modes to test.
- The semantics are trivially predictable from the YAML alone.
- Cost: no routine survives its pinned host being down, and adding leases later
  introduces a second, coordinated mode whose semantics diverge from everything shipped
  in v1.

---

## ADR-0208 — Routine definitions are git-shared; scheduler state is host-local and never synced

**Status:** Accepted · 2026-07 · [ORB-10021]

### Context

Routines run on two hosts (dk-mac, dk-server-1) with different availability profiles.
Definitions must converge across hosts; scheduler runtime state (last fires, pauses,
locks) could either be synced between hosts or kept local. Syncing state would let either
machine answer "did the nightly fire on the other box?" but requires a scheduler network
protocol between hosts that only expose 22/443 to each other.

### Decision

Routine YAML definitions live in routine-source workspaces and converge via git like any
other versioned definition. All scheduler state — fires (with `name + slot + attempt`
idempotency keys), host-local pauses, and the sweep lock — lives in host-local storage
(the `routine_*` tables in `~/.orbit/orbit.db`, plus a `flock(2)` sweep lock that the OS
releases on process death), gitignored and never synced. No scheduler network protocol
exists in v1.

### Consequences

- Two hosts converge on definitions through a normal `git pull`; no new sync mechanism to
  build, secure, or debug.
- State stays consistent with the run history it references, which is also host-local.
- Cost: cross-host observability requires asking each host — there is no single pane of
  glass, and a definition edit is only as fresh on the other host as its last `git pull`.

---

## ADR-0215 — Default routines seed per-workspace at init with host and name resolved at seed time

**Status:** Accepted · 2026-07 · [ORB-10129]

### Context

ORB-10129 ships the failed-run triage pipeline as a first-class default, but routines
have no global directory: discovery reads `.orbit/routines/*.yaml` from
`[routines] role = "source"` workspaces (ADR-0205), v1 requires explicit host pinning
with no "any host" (ADR-0207), and names must be unique across all sources on a host —
so a static shipped YAML cannot work. The real alternatives were leaving the triage
routine workspace-authored (no out-of-the-box self-healing) or adding a global routines
directory (a discovery-model change ADR-0205 deliberately avoided).

### Decision

`orbit init` (workspace branch) seeds `DEFAULT_ROUTINE_FILES` templates into
`.orbit/routines/`, resolving `__ORBIT_HOST_ID__` via `resolve_host_id` and
`__ORBIT_ROUTINE_NAME__` from a workspace-directory slug (`task-triage-<workspace>`),
validating each rendered document fail-closed before writing. Plain re-init preserves
user edits while creating newly introduced missing defaults; destructive `--force`
recreates templates. Every default is seeded with `enabled: false`, so a routine fires
only after the workspace is a routine source and its versioned enable switch is set true.

### Consequences

- A fresh workspace gets reviewable definitions without silently granting scheduled
  execution; each routine is enabled explicitly in version control.
- Per-workspace names let multiple seeded source workspaces coexist on one host despite
  the global name-uniqueness rule.
- The seeded file pins the initializing host; sharing the repo to another host needs a
  re-init with `--refresh-defaults` or a hand edit of `hosts:`.
- Cost: `orbit init` output now depends on the machine it runs on (host id, directory
  name) — two clones of the same repo can carry differently-rendered routine files, and
  identical workspace directory names on one host still collide fail-closed.

---

## ADR-0223 — Delegate workspace ship routines through a synchronous wrapper job

**Status:** Accepted · 2026-07 · [ORB-10207]

### Context

A scheduled ship routine must dispatch only its source workspace, resolve that workspace's
ship mode and base branch, and keep the parent run active until normal backlog shipment
finishes. The alternatives were special-casing routine dispatch, spawning the legacy
multi-workspace CLI sweep, or composing the existing job catalog.

### Decision

Seed a workspace-local `ship_sweep` routine targeting `workspace_ship_pipeline`. The
wrapper deterministically resolves ship input for its active runtime, invokes and waits
for `task_auto_pipeline` with no explicit task IDs, and guards child success. It does not
consult `workflow.auto_ship` or the cross-workspace sweep path.

### Consequences

- Backlog discovery, readiness, locking, bundling, crew selection, and gates remain owned
  by `task_auto_pipeline`; an empty backlog remains a clean no-op.
- `overlap: forbid` covers child shipment because the wrapper remains active while waiting.
- The legacy global ship-sweep remains compatible during burn-in but is not used by routines.
- Cost: the catalog gains a wrapper job and deterministic resolver activity whose input
  contract must stay aligned with the canonical ship workflow.

---

## Task References

- [ORB-10001] — authored this design-doc folder (proposal).
- [ORB-10021] — implemented routines v1; allocated and accepted ADR-0204..ADR-0208.
- [ORB-10129] — shipped the default triage routine; allocated and accepted ADR-0215.
- [ORB-10207] — seeded disabled defaults and allocated/accepted ADR-0223 for workspace ship.
- [ORB-10138] — exposed per-routine scheduler health over the dashboard HTTP API
  (`GET /api/routines`), realizing the single-host half of the §7 cross-host-visibility
  vision. Read-only projection of `routine_statuses`; no new ADR (no new architectural
  constraint — mirrors the existing `orbit routine list --json` surface).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
