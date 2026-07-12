---
title: Garbage Collection — Vision
owner: codex
last_updated: 2026-07-12
status: Draft
feature: gc
doc_role: vision
type: design
summary: Tracks future GC questions beyond the v1 explicit, safety-first retention contract.
tags: [gc, retention, safety]
paths: ["crates/orbit-cli/src/command/gc/**", "crates/orbit-core/src/command/gc/**"]
related_features: [gc, activity-job, auditability, task-artifacts, worktree-artifacts]
related_artifacts: [ORB-10178, ADR-0220]
---

# Garbage Collection — Vision

The v1 direction is deliberately conservative: explain first, apply explicitly,
and retain anything whose ownership or liveness is uncertain. This document
tracks possible extensions; none weakens the normative invariants in
[2_design.md](./2_design.md).

## 1. Open Questions

1. Can the host-wide apply lock eventually become a hierarchy without allowing
   cross-workspace races over global logs, skills, audit data, or registries?
2. Which platforms can provide a common descriptor-relative, no-follow deletion
   primitive with equally strong revalidation guarantees?
3. Should a signed plan be exportable for separate human approval and later
   apply, and what maximum age/version checks would prevent stale authority?
4. What export, tombstone, restore, and relation model would make physical task
   purge safe enough to propose?
5. Can allocated disk blocks be measured portably enough to supplement logical
   byte estimates without making collection depend on platform utilities?

## 2. Prior Work

### Orbit cleanup surfaces

`orbit audit prune`, JSONL startup pruning, skill unlink/init cleanup, and the
archived worktree-GC attempt [ORB-10173] provide useful domain primitives. They
also demonstrate why command ownership, mutation gating, live-owner checks, and
one report contract must be established before reuse.

### Storage lifecycle systems

Database vacuuming, object-store lifecycle policies, Git worktree management,
and tracing-log rotation all separate eligibility from physical reclamation in
different ways. Orbit borrows their source-of-truth clocks and restart-safe
thinking but cannot delegate cross-domain ownership or referential integrity to
any one of them.

## 3. What May Be Distinctive

The individual mechanisms are conventional. The potentially distinctive part
is treating heterogeneous cleanup as one explainable plan whose safety
invariants are stronger than its retention settings: automation and operators
can shorten age thresholds, but neither can convert uncertainty into permission.

## 4. References

**Orbit-internal**

- [Garbage Collection design](./2_design.md)
- [Activity / Job design](../activity-job/2_design.md)
- [Auditability design](../auditability/2_design.md)
- [Task Artifacts design](../task-artifacts/2_design.md)
- [Worktree Artifacts design](../worktree-artifacts/2_design.md)

**External**

- Git documentation for `git worktree remove` and `git worktree prune`.
- SQLite documentation for transactions and write-ahead logging.

## Task References

- [ORB-10173] — demonstrated the worktree leak and an earlier domain-specific GC attempt.
- [ORB-10178] — defined the GC retention and safety contract.
- [ORB-10189] — will validate aggregate and opt-in automatic collection.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
