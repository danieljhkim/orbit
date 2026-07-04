---
title: Routines — Vision
owner: claude
last_updated: 2026-07-03
status: Draft
feature: routines
doc_role: vision
type: design
summary: Open questions and prior art for the routines scheduler — leases, event triggers, ship-sweep convergence.
tags: [routines, scheduler]
paths: ["crates/orbit-core/src/routines/**"]
related_features: [routines, activity-job]
related_artifacts: [ORB-10001]
---

# Routines — Vision

Forward-looking questions for the routines feature. Everything here is explicitly *not*
part of the v1 contract in [2_design.md](./2_design.md); items graduate by getting a task
and an ADR, not by drifting in.

---

## 1. Open Questions

1. **Single-fire across hosts.** v1 pins routines to explicit hosts. A "exactly one of N"
   mode needs a lease: the natural v2 shape is a lease table in one designated host's store,
   reached over SSH (port 22 is the only always-open channel between the current hosts).
   Worth doing only when a real routine needs failover, not before.
2. **Folding `ship-sweep` into a routine.** `orbit run ship-sweep` predates routines and
   hard-codes its own opt-in (`[workflow] auto_ship`). Once routines ship, ship-sweep is
   expressible as a seeded routine targeting the backlog-dispatch job. Convergence would
   retire one bespoke scheduler entrypoint — but only after routines have run unattended
   long enough to be trusted with it.
3. **Event triggers.** File-watch, webhook, or run-completion triggers ("reindex after any
   docs change") require a resident process — the thing v1 deliberately avoids. If bridge
   ever grows a long-lived daemon on the always-on box, it may be the natural event source,
   with routines subscribing rather than Orbit growing its own daemon.
4. **Routine-emitted tasks.** A routine whose job files an Orbit task on findings (nightly
   drift check → task per drift) works today via job semantics; what's open is whether
   routines should get first-class dedup support ("don't file a duplicate of an open task
   from a previous fire") or leave that to job logic.
5. **Sub-minute and jitter.** Minute granularity is a v1 floor. Per-routine jitter matters
   only if many routines land on the same slot and contend; revisit when there are enough
   routines for it to be observable.
6. **Missed-run variants.** `catch_up_once | skip` covers current needs; a count-preserving
   `catch_up_all` (anacron-style) is additive if a routine ever needs per-slot semantics.
7. **Cross-host visibility.** Each host's state is local, so "did the nightly commit fire
   on the other box?" requires asking that box. A read-only aggregation surface (dashboard
   projection or bridge tool querying both stores) is open; state *sync* is explicitly not
   the answer.

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
  ordinary Orbit runs with audit envelopes, linkable to tasks and learnings — the scheduler
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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
