---
title: Resident Orchestrator — Vision
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: vision
type: design
summary: After the drain job and workspace sequencer exist, later work may add an Orbit-owned clock, epic-scoped scans, or conversation resume.
tags: [resident-orchestrator, epic, jobs]
paths: [".orbit/resources/jobs/**"]
related_features: [resident-orchestrator, routines]
related_artifacts: [ORB-10775, ORB-10788]
---

# Resident Orchestrator — Vision

V1 proves that a scan-and-drain job plus an external clock is enough. [ORB-10788]
adds a one-tick sequencer so auto logistics do not live inside leaf ship. The items
below stay out of [ORB-10775] and [ORB-10788].

## 1. Open Questions

1. **Should Orbit grow a routine that fires `epic_pipeline` or `workspace_auto_pipeline`?**
   Only after the jobs are boring in production. A seeded routine is how this becomes
   a resident server by accident. Retargeting `workspace_ship_pipeline` is not a new
   routine.
2. **Should the scan be epic-scoped?** A `parent_id` / `epic` tag filter would let one
   workspace hold several bodies of work. The sequencer's "one epic at a time after
   leaves" heuristic is the interim policy; do not teach `scan_unresolved_work` that
   filter until the heuristic is boring.
3. **Conversation resume?** Session log is v1 memory. Resume is still allowed later as
   a fail-open optimization, never as the notebook.
4. **Event-driven wake?** A task-created or run-failed hook would cut cron latency. Routines
   v1 have no event trigger; do not add one for this feature.
5. **Multiple orchestrator identities per workspace?** Requires a routing key. Tags alone
   are not enough.

## 2. Prior Work

The first draft of this folder specified an in-Orbit resident (CLI session, decision
comments, `select_resident_epic`, seeded routine). Polar is `orchestrator/reconciler.md`
already argued the opposite: a stateless tick over Orbit + runs, no second store. V1 takes
the tick (the scan + loop) and leaves the clock outside.

## 3. What May Be Distinctive

The drain predicate is boring on purpose: three task statuses and two run states. The
orchestrator is a normal CLI agent with tools it already has. The only new contract is
that the job, not the model, decides whether work remains.

## 4. References

- [Design](./2_design.md)
- Polar is `design/orchestrator/reconciler.md` (constellation tree; not an Orbit ADR)

## Task References

- **[ORB-10775]** — v1 drain epic. Items in §1 stay out.
- **[ORB-10788]** — Sequencer. Does not close §1.2 (epic-scoped scan).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
