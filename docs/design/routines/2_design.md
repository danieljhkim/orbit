---
title: Routines — Design
owner: claude
last_updated: 2026-07-05
status: Accepted
feature: routines
doc_role: design
type: design
summary: Proposed contract for routine definitions, sweep dispatch, host-local state, and OS clock integration.
tags: [routines, scheduler]
paths: ["crates/orbit-cli/src/command/routine/**", "crates/orbit-core/src/routines/**"]
related_features: [routines, activity-job]
related_artifacts: [ORB-10001, ORB-10021]
---

# Routines — Design

This doc is the v1 contract as shipped in [ORB-10021]: the routine definition schema,
how definitions are discovered, what `orbit sweep` does on each invocation, where state
lives, and how the OS clock drives it. Cross-host coordination, event triggers, and everything else deferred is
in [3_vision.md](./3_vision.md). Decision rationale lives in [4_decisions.md](./4_decisions.md).

---

## 1. Routine Definition

A routine is one YAML file under `.orbit/routines/` in a routine-source workspace,
PR-reviewed and versioned like any other shared definition.

```yaml
# .orbit/routines/almanac-auto-commit.yaml
schemaVersion: 1
name: almanac-auto-commit
description: Commit & push almanac changes nightly
enabled: true                  # global kill-switch, versioned
hosts: [dk-mac]                # explicit host pinning; no "any host" in v1
trigger:
  cron: "0 22 * * *"           # standard 5-field cron, evaluated in host-local time
  missed_run: catch_up_once    # catch_up_once | skip (default: skip)
target: job:almanac_commit_pipeline   # job:<name>, resolved via the catalog
policy:
  timeout_minutes: 10
  retries: { max: 2, backoff_minutes: 2 }
  overlap: forbid              # forbid | allow — forbid skips a fire while one is in flight
```

Field semantics:

- **`name`** — unique across all routine sources on a host; collision is a load-time error.
- **`enabled` / `hosts`** — the two *versioned* toggle layers. A routine fires on a host only
  if `enabled: true` and the host's `host_id` appears in `hosts`. Effective state also
  requires no host-local pause (§4).
- **`trigger.cron`** — when the routine is due. `missed_run` governs fires that fall in a
  window when the host was asleep or powered off: `catch_up_once` fires a single make-up run
  on the next sweep (never one per missed slot); `skip` waits for the next natural slot.
- **`target`** — a `job:<name>` reference resolved through the source workspace's job
  catalog at load time. Unresolvable targets are load-time errors. `activity:<name>` is
  reserved and rejected at parse time with wrapping guidance: run dispatch is job-shaped
  (`submit_pipeline_run` resolves jobs by name; nothing dispatches a bare activity), and a
  one-step wrapper job in the same source workspace is the existing composition grammar
  ([ADR-0206]). There is deliberately no inline command form: the `shell` activity variant
  was removed fail-closed in [ORB-00374] / [ADR-0194], and reintroducing arbitrary-command
  payloads through the scheduler would reopen that surface on a timer.
- **`policy`** — applied by the dispatcher around the run: timeout, bounded retries with
  fixed backoff, and overlap handling. `overlap: forbid` is the default; `timeout_minutes`
  defaults to 60 and doubles as the staleness horizon (§4), `retries` defaults to
  `{max: 0, backoff_minutes: 2}`. Retries re-dispatch a *failed* fire under the same slot
  (attempt 2..max+1) once the backoff has elapsed, evaluated on later sweep passes. A fire
  that *errored at dispatch* (`submit_pipeline_run` failed synchronously — lock contention, a
  momentary catalog/store hiccup) is retried under the same policy as a run-level failure
  ([ORB-00422]): a transient dispatch failure is the class retries exist to absorb, so it must
  not burn the slot when retry budget remains. With `max: 0` (the default) a dispatch error,
  like any failure, consumes the slot and waits for the next natural one.

Parsing is fail-closed: an invalid routine file is reported and *that routine* is treated
as absent; it never degrades into "fire with defaults".

---

## 2. Discovery and Registration

Discovery reuses the global workspace registry (`~/.orbit/workspaces.json`) rather than a
new pointer mechanism — the same shape `orbit run ship-sweep` established for unattended
cross-workspace dispatch.

A workspace becomes a routine source with one versioned config key:

```toml
# <workspace>/.orbit/config.toml
[routines]
role = "source"
```

On each pass, sweep loads the registry, visits every registered, active workspace whose
config declares `role = "source"`, and loads `.orbit/routines/*.yaml` from each. Two
properties fall out:

- **Registration is what already exists.** Registering the workspace with Orbit (which
  polaris needs anyway) plus the config key is the entire setup; the key is versioned, so
  both hosts converge on it through a normal `git pull` with no per-host pointer files.
- **Centralization is convention, not mechanism.** The constellation keeps all routines in
  polaris; the mechanism tolerates additional sources, and `orbit routine list` names each
  routine's source workspace so provenance is never ambiguous.

Host identity is the one genuinely host-local datum: `~/.orbit/host.toml` carries
`host_id = "dk-mac"`, defaulting to the machine hostname when absent. `orbit routine init`
writes it and (with `--install-clock`) installs the OS clock unit (§5). A malformed
`host.toml` is an error, not a fallback; a `[routines] role` value other than `"source"`
is a config error (fail-closed on both).

---

## 3. Sweep

`orbit sweep` is the stateless entrypoint the OS clock invokes every minute. Like
`ship-sweep`, it never bootstraps a workspace from the caller's cwd, isolates per-routine
failures, and exits non-zero only on infrastructure errors (registry unreadable, store
unopenable) — an unconfigured host logs one line and exits 0, because launchd/systemd will
invoke it forever and an unconfigured host is not an error state.

Per pass:

1. Take a host-global advisory lock (in the host store, §4). If another sweep holds it,
   exit immediately — overlapping invocations from a slow prior pass must not double-fire.
2. Load the registry; collect routines from all source workspaces (fail-closed per file).
3. Filter to routines where `enabled`, `hosts` contains this `host_id`, and no local pause.
4. Sync unresolved fires against actual run state, reclaiming entries older than the
   routine's `timeout_minutes` (the staleness horizon — a sweep that crashed between
   intent and dispatch must not block `overlap: forbid` forever).
5. For each, compute due-ness from the cron expression and the persisted cursor
   (last slot, else the first-observation baseline — a routine never fires for slots that
   predate its registration on this host; the first sweep records the baseline and fires
   nothing). Due-ness is O(1) via previous-occurrence lookup, never a walk over every
   missed slot; `missed_run` policy decides gaps. A slot is "natural" within a 120s grace
   of its scheduled time.
6. For each due routine: check `overlap` against in-flight fires, record the fire intent
   (idempotency key: routine name + scheduled slot + attempt, transactionally with the
   cursor advance), then dispatch the target via `submit_pipeline_run` in the routine's
   source workspace with actor `routine/<name>` as run provenance.
7. Record outcomes and exit.

Fires are normal runs: they appear in run history, carry v2 audit envelopes, and are
debuggable with the existing run tooling — there is no separate "scheduled run" ledger.

Naming note: `orbit sweep` is the general scheduler pass; `orbit run ship-sweep` remains
the shipped, single-purpose backlog sweep. Folding ship-sweep into a seeded routine is a
vision item ([3_vision.md](./3_vision.md) §1), not a v1 goal.

---

## 4. Host-Local State and Toggles

All scheduler state lives host-locally (the `routine_*` tables in the host-global store
database `~/.orbit/orbit.db`, module `orbit-store/src/sqlite/routine_store/`), gitignored
and never synced:

- **routine_cursors** — per routine: first-observation baseline + last slot consumed.
- **routine_fires** — one row per fire attempt: `(name, slot, attempt)` idempotency key,
  state (`intent → dispatched → succeeded/failed/timed_out/error`), dispatched run id.
- **routine_pauses** — host-local suppressions written by `orbit routine pause <name>` /
  cleared by `resume`. Durable across reboots; invisible to git.
- **sweep lock** — a `flock(2)` file lock (`~/.orbit/state/routine-sweep.lock`) rather
  than a table: the OS releases it on process death, so a crashed sweep never wedges the
  next pass and no lock-staleness logic is needed.

Toggle resolution, in order: `enabled: false` (versioned, everywhere) → not in `hosts`
(versioned, per host) → local pause (unversioned, this host only). `orbit routine list`
shows all three columns plus computed next-due, so "why didn't this fire?" is one command.

---

## 5. Clock Integration

The OS owns the wake-up; Orbit owns everything else. `orbit routine init --install-clock`
renders and installs the platform unit:

- **macOS** — a launchd agent (`com.orbit.sweep`) with `StartInterval` 60s. launchd also
  fires on wake, which pairs with `missed_run: catch_up_once` for laptop sleep gaps.
- **Linux** — `orbit-sweep.timer` (`OnCalendar=*:*:00`, `Persistent=true`) plus a oneshot
  service. `Persistent=true` covers downtime at the OS layer; per-routine `missed_run`
  still decides whether the covered gap produces a make-up fire.

There is no resident Orbit daemon. Sub-minute triggers and event triggers are explicitly
out of v1 scope for this reason.

---

## 6. Concerns & Honest Limitations

- **No cross-host coordination.** `hosts` pins explicitly; a routine listed on both hosts
  runs on both, independently. "Exactly one of N hosts" requires a lease protocol across a
  tailnet that only exposes 22/443 between these machines — deferred, additive if needed.
- **Definition staleness.** Sweep reads whatever revision of the source workspace is on
  disk; definitions are only as fresh as the last `git pull`. A pull-the-sources routine
  can narrow the window but cannot fix its own staleness (it, too, is a definition). Editing
  routines on the host that runs them has no staleness; the other host lags by one sync.
- **Scheduled execution is a real capability escalation.** A routine source workspace is
  scheduled code execution on every host that trusts it. Targets are catalog-resolved (no
  inline commands) and run under existing activity/job policy, but note the sandbox caveat
  recorded in [ADR-0196]: enforcement depends on which runtime path the target takes.
  PR review on the source workspace is part of the security boundary.
- **Minute granularity, host-local time.** Cron is evaluated in host-local time; DST folds
  can skip or double a slot exactly as classic cron does. The idempotency key (name + slot)
  prevents double *fires* for the same slot but cannot invent a skipped slot.
- **`catch_up_once` collapses history.** After a week of laptop sleep, a nightly routine
  fires once, not seven times. This is the intended semantic for every currently envisioned
  routine (commits, reindexing), but it is a lossy default; a count-preserving mode would
  be a new `missed_run` variant.
- **First minute of overlap risk is on the dispatcher.** `overlap: forbid` depends on
  accurate in-flight bookkeeping; a crashed sweep that recorded a fire intent but died
  before dispatch leaves a stale in-flight entry. The outcome-sync step reclaims intents
  and dispatches older than `policy.timeout_minutes` (marking them `error` / `timed_out`);
  the consumed slot is not re-fired — the idempotency key holds. The two failure paths are
  deliberately given different states so retry can tell them apart ([ORB-00422]): a
  *synchronous* `submit_pipeline_run` error is unambiguous (nothing dispatched), so `fire`
  records it as `failed` — retryable under `policy.retries` like a run-level failure; a crashed
  sweep's reclaimed stale intent is *ambiguous* (a worker may have partially started), so the
  outcome sync records it as `error` — terminal, never re-fired, so a make-up fire cannot race
  an orphaned run.
- **Routines carry no input payload.** v1 dispatches every target with an empty input
  object; jobs meant for routines must run with defaults. Parameterized fires would be a
  schema addition.

---

## Task References

- [ORB-10001] — authored this design-doc folder (proposal; no implementation).
- [ORB-10021] — implemented routines v1 (types, store, sweep, CLI, clock units).
- [ORB-00374] — removed the `shell` activity variant and `run_shell` dispatch (fail-closed);
  routines inherit this constraint.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
