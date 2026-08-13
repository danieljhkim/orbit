---
title: Routines — Vision
owner: claude
last_updated: 2026-07-18
status: Draft
feature: routines
doc_role: vision
type: design
summary: Open questions and prior art for the routines scheduler — leases, event triggers, ship-sweep convergence.
tags: [routines, scheduler]
paths: ["crates/orbit-core/src/routines/**", "crates/orbit-remote/src/routines.rs"]
related_features: [routines, activity-job, host-registry]
related_artifacts: [ORB-10001, ORB-10021, ORB-10207, ORB-10270, ORB-10319, ADR-0223]
---

# Routines — Vision

Forward-looking questions for the routines feature. Everything here is explicitly *not*
part of the v1 contract in [2_design.md](./2_design.md); items graduate by getting a task
and an ADR, not by drifting in.

---

## 1. Open Questions

0. **First-class `activity:` targets.** v1 rejects `activity:<name>` at parse time because
   run dispatch is job-shaped ([ADR-0206]); the wrapper-job idiom covers current needs. A
   standalone activity run entrypoint (or auto-wrapping) would let routines fire
   activities directly — worth doing only if the wrapper friction proves real.
1. **Single-fire across hosts.** v1 pins routines to explicit hosts. A "exactly one of N"
   mode needs a lease: the natural v2 shape is a lease table in one designated host's store,
   reached over SSH (port 22 is the only always-open channel between the current hosts).
   Worth doing only when a real routine needs failover, not before.
2. **Event triggers.** File-watch, webhook, or run-completion triggers ("reindex after any
   docs change") require a resident process — the thing v1 deliberately avoids. If bridge
   ever grows a long-lived daemon on the always-on box, it may be the natural event source,
   with routines subscribing rather than Orbit growing its own daemon.
3. **Routine-emitted tasks.** A routine whose job files an Orbit task on findings (nightly
   drift check → task per drift) works today via job semantics; what's open is whether
   routines should get first-class dedup support ("don't file a duplicate of an open task
   from a previous fire") or leave that to job logic.
4. **Sub-minute and jitter.** Minute granularity is a v1 floor. Per-routine jitter matters
   only if many routines land on the same slot and contend; revisit when there are enough
   routines for it to be observable.
5. **Missed-run variants.** `catch_up_once | skip` covers current needs; a count-preserving
   `catch_up_all` (anacron-style) is additive if a routine ever needs per-slot semantics.
6. **Cross-host visibility.** Each host's state is local, so "did the nightly commit fire
   on the other box?" requires asking that box. The single-host half of this is now built:
   `GET /api/routines` projects this host's routine health (last fire, outcome, duration,
   next due) over the dashboard HTTP API [ORB-10138], so a stopped sweep is visible remotely
   without box ssh. True cross-host *aggregation* (one surface querying every box's store)
   remains open; state *sync* is still explicitly not the answer.

### Graduated

- **Workspace-local ship-sweep convergence ([ORB-10207], [ADR-0223]).** The default
  `ship_sweep` routine delegates synchronously to the normal shipment pipeline for only
  its source workspace. It is seeded disabled and enabled through the versioned
  definition. The legacy global CLI entrypoint remains during burn-in; removing it is a
  separate compatibility task.

---

## 2. Prior Work

### OS schedulers
- **cron / anacron** — the trigger vocabulary (5-field expressions) and the missed-run
  problem anacron exists to solve; routines adopt the vocabulary and make missed-run policy
  per-definition instead of system-wide.
- **systemd timers / launchd** — v1's actual clock. `Persistent=true` and launchd wake
  behavior are load-bearing; routines deliberately delegate "when does the machine wake"
  to them.

### Workflow engines
- **Temporal / Cadence schedules** — durable schedules attached to durable executions,
  with overlap policies (`skip`, `buffer_one`) that v1's `overlap: forbid` and
  `catch_up_once` consciously echo at much smaller scale. The full replayable-execution
  model is what this feature deliberately does *not* adopt.
- **Kubernetes CronJob** — `concurrencyPolicy`, `startingDeadlineSeconds`, and the
  documented pain of missed-fire semantics; a compact catalog of the edge cases §6 of
  [2_design.md](./2_design.md) must test.

### CI schedulers
- **GitHub Actions `schedule:`** — git-versioned schedule definitions co-located with the
  code they operate on; the definition-review-as-security-boundary posture routines share.

---

## 3. What May Be Distinctive

- **Git-versioned, PR-reviewed schedules over a knowledge-integrated runtime.** Fires are
  ordinary Orbit runs with audit envelopes, linkable to tasks — the scheduler
  and the knowledge system share one substrate.
- **Agent-invoking targets.** A routine can fire an `agent_loop` activity: scheduled agent
  work (nightly triage, periodic research) with the same policy and audit surface as any
  other run — most schedulers fire commands; this one fires accountable agent runs.
- **Definitions-shared / state-local as a stance.** Two hosts converge through git alone;
  there is no scheduler network protocol at all in v1.

---

## 4. References

Orbit-internal:
- [../activity-job/1_overview.md](../activity-job/1_overview.md) — the execution substrate
  routines trigger into.
- [../activity-job/4_decisions.md](../activity-job/4_decisions.md) — [ADR-0194], the
  removed-shell posture routines inherit.
- [../executors/4_decisions.md](../executors/4_decisions.md) — [ADR-0196], sandbox caveats
  relevant to what scheduled targets may do.

External:
- systemd.timer(5), launchd.plist(5) — persistence and wake semantics.
- Temporal "Schedules" documentation — overlap/catch-up policy vocabulary.
- Kubernetes CronJob documentation — concurrency and missed-fire edge cases.

---

## Task References

- [ORB-10001] — authored this design-doc folder (proposal; no implementation).
- [ORB-10021] — implemented routines v1.
- [ORB-10207] — graduated workspace-local ship-sweep scheduling from this vision.
- [ORB-10319] — separated Remote registry/runtime composition from Core's scheduler kernels without changing these open questions.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
