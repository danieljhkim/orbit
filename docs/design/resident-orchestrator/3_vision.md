---
title: Resident Orchestrator — Vision
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: vision
type: design
summary: Forward-looking questions for multi-resident routing, event-driven wakeups, and cross-workspace epic observability.
tags: [resident-orchestrator, epic, routines, cli]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**"]
related_features: [resident-orchestrator, routines, host-registry, mcp-bridge]
related_artifacts: [ORB-10775, ADR-0361]
---

# Resident Orchestrator — Vision

This document records directions that are intentionally outside the first CLI-backed resident
cycle. The first release should prove that one workspace, one resident, one active epic, and
durable task state are sufficient before adding routing or coordination machinery.

## 1. Open Questions

1. **Should pickup become event-driven?** Routines v1 deliberately use an OS clock and have no
   event triggers. A future task-created event could reduce latency, but it would introduce a
   resident process, webhook, or durable event consumer that the first design avoids.
2. **Can one workspace support several residents?** Multiple specialists would require an explicit
   routing key, capability declaration, and lease/claim semantics. Workspace plus `epic` is only
   unambiguous while ownership is singular.
3. **How should cross-workspace outcomes roll up?** A product epic spanning several repositories may
   need a top-level constellation task that relates workspace-local epics without pretending their
   local `parent_id` trees share one store.
4. **Should resident health become first-class?** Operators may eventually need last successful
   cycle, current epic, stalled duration, and identity/config mismatch surfaced alongside routine
   health.
5. **What is the right checkpoint artifact?** Parent comments are sufficient for v1. A structured
   resident-cycle artifact could improve dashboards and replay, but risks duplicating task and run
   state.
6. **Should identity be promoted beyond an activity asset?** If many products need resident agents,
   a typed resident profile may become worthwhile. It should be justified by repeated configuration
   drift, not introduced before the activity-based canary.
7. **Should residents pick up proposed epics?** V1 requires an upstream authority to move an epic
   to `backlog`; it deliberately has no implicit proposed-work authority. A future version could
   add an explicit policy block to the routine or workspace configuration, but only once its
   approval provenance and operator visibility are defined.

## 2. Prior Work

### Orbit routines and jobs

Routines already establish the desired scheduler boundary: the OS owns the clock, Orbit performs a
stateless sweep, and a versioned routine targets a catalog job. Activity / Job provides CLI agent
invocation and deterministic control flow. Resident orchestration composes those primitives rather
than adding another scheduler.

### Actor and queue systems

Actor mailboxes and work queues demonstrate that an address plus a durable message can decouple
senders from owners. The resident design borrows that separation but keeps the address at workspace
granularity and the message as an ordinary Orbit task, avoiding another queue schema.

### Hierarchical planning agents

Planner/executor systems commonly decompose goals into leaf work. Orbit's distinction is intended
to be operational rather than algorithmic: decomposition becomes parent/child tasks and
dependencies, while shipment, review, and merge remain independently auditable workflows.

## 3. What May Be Distinctive

The individual ingredients are conventional. The potentially useful combination is that agent
delegation, durable ownership, decomposition, and delivery evidence all reuse the repository's
existing task and workflow model. The resident has a durable home and a specialized identity, but
there is still no resident agent server. A CLI process can disappear after every cycle without
losing the meaning or current state of the assignment.

## 4. References

**Orbit-internal**

- [Resident Orchestrator design](./2_design.md)
- [Activity / Job overview](../activity-job/1_overview.md)
- [Routines design](../routines/2_design.md)
- [Host Registry design](../host-registry/2_design.md)
- [MCP Bridge design](../mcp-bridge/2_design.md)

**External**

- [The Actor Model](https://www.microsoft.com/en-us/research/publication/actors-a-model-of-concurrent-computation-in-distributed-systems/)
- [Temporal durable execution](https://docs.temporal.io/temporal)

## Task References

- **[ORB-10775]** — v1 implementation epic. Items in §1 stay out of that epic.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
