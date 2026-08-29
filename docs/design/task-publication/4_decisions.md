---
title: Task Publication — Decisions
owner: codex
last_updated: 2026-08-29
last_validated: 2026-08-29
status: Draft
feature: task-publication
doc_role: decisions
type: design
summary: Standing authority, failure, and data-exposure rules for task publication.
tags: [task-publication, task-artifacts, backup, git, multi-host]
paths: ["crates/orbit-store/src/workflow/task/**", "crates/orbit-registry/**"]
related_features: [task-publication, task-artifacts, remote-access, federated-mcp, host-registry]
related_artifacts: [ORB-11068]
---

# Task Publication — Decisions

These standing rules govern future implementation choices for task publication.
Task references carry provenance; superseded decisions remain in place so their
original reasoning stays legible. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-decisions) for the admission rule.

## Publication never creates a second mutation authority

**Recorded:** 2026-08-29 · [ORB-11068]

### Context

A Git snapshot is portable and writable by repository tooling, which makes it
easy for a future consumer to treat a fetched tree as another live task store.
Doing so would create competing allocators and lifecycle writers without also
replicating locks, claims, runs, audit, or execution ownership.

### Decision

For every future publication consumer, fetching, inspecting, caching, and
restoring a snapshot remain distinct from authority. Only the registry-declared
workspace owner may mutate canonical tasks or advance the publication ref.

### Consequences

- Read-only consumers may exist on any number of machines without becoming
  replicas of the control plane.
- New offline-write features must define authority transfer or a complete
  multi-writer contract before changing this rule.
- Cost: a consumer cannot continue normal task mutation while the owner is
  unavailable merely because it has a recent publication.

## Publication does not participate in task-write acknowledgement

**Recorded:** 2026-08-29 · [ORB-11068]

### Context

Putting fetch, commit, and push on every task mutation would make local task
durability depend on network reachability and Git credentials. A backup channel
would become a synchronous coordinator.

### Decision

Future task publication is explicit or routine-driven derived work. A task write
is acknowledged by the canonical local store; publication records and reports
its own last-success generation independently.

### Consequences

- Network and remote-host failures do not block task lifecycle operations.
- Operators can select a publication frequency proportional to their recovery
  objective.
- Cost: the newest acknowledged task mutation may be absent from the last remote
  publication.

## A moved publication ref is an authority conflict

**Recorded:** 2026-08-29 · [ORB-11068]

### Context

Git can merge or force-update divergent histories, but either response would
hide the fact that more than one writer advanced an authority-owned durability
ref.

### Decision

Every future publisher must compare-and-swap against the fetched remote tip. A
non-fast-forward result stops publication and surfaces an authority conflict;
the implementation never merges, replays task operations, or force-pushes as an
automatic recovery.

### Consequences

- Out-of-band and competing writes fail visibly at the ref boundary.
- The orphan history stays linear and each snapshot names an unambiguous
  predecessor.
- Cost: benign manual edits and stale authority handoffs require operator
  reconciliation before publication resumes.

## Attached bytes require an explicit exposure policy

**Recorded:** 2026-08-29 · [ORB-11068]

### Context

Task attachments may contain screenshots, logs, traces, environment details,
credentials, or high-churn binary data. Publishing them to the code remote makes
their bytes durable in Git history and may broaden who can access them.

### Decision

Before first publication, every future implementation requires an explicit
attachment policy. Inclusion enforces independent path, sensitivity, per-file,
and total-size checks. Omission is recorded as incomplete backup state, never
silently treated as full recovery coverage.

### Consequences

- Operators make the durability-versus-exposure tradeoff consciously.
- Recovery can state exactly which attachment bytes were not protected.
- Cost: the safest default may refuse publication until the operator configures
  how existing attachments should be handled.

## Task References

- [ORB-11068] — recorded the standing authority, acknowledgement, ref-conflict, and attachment-policy rules.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
