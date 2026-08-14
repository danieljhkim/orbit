---
title: Resident Orchestrator — Vision
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: vision
type: design
summary: After the drain job exists, later work may add an Orbit-owned clock, epic-scoped scans, or conversation resume — none of that is v1.
tags: [resident-orchestrator, epic, jobs]
paths: [".orbit/resources/jobs/**"]
related_features: [resident-orchestrator, routines]
related_artifacts: [ORB-10775, ADR-0362]
---

# Resident Orchestrator — Vision

V1 proves that a scan-and-drain job plus an external clock is enough. The items below stay
out of [ORB-10775].

## 1. Open Questions

1. **Should Orbit grow a routine that fires `epic_pipeline`?** Only after the job is
   boring in production. A seeded routine is how this becomes a resident server by
   accident.
2. **Should the scan be epic-scoped?** A `parent_id` / `epic` tag filter would let one
   workspace hold several bodies of work. That needs a selector policy the first draft
   tried to invent inside Orbit; keep it in the supervisor until the drain job is real.
3. **Conversation resume?** Useful once drains are long and frequent. Not a correctness
   dependency; fail-open to a fresh invoke.
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

- **[ORB-10775]** — v1 epic. Items in §1 stay out.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
