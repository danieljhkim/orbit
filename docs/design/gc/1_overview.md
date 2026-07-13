---
title: Garbage Collection — Overview
owner: codex
last_updated: 2026-07-12
status: Draft
feature: gc
doc_role: overview
type: design
summary: Defines Orbit's explainable, explicitly applied retention family across global and workspace state.
tags: [gc, retention, safety]
paths: ["crates/orbit-cli/src/command/gc/**", "crates/orbit-core/src/command/gc/**", "crates/orbit-core/src/config/**"]
related_features: [gc, activity-job, auditability, task-artifacts, worktree-artifacts]
related_artifacts: [ORB-10178, ORB-10181, ORB-10183, ADR-0220]
---

# Garbage Collection — Overview

`orbit gc [TARGET]` is the single inspection and retention family for
Orbit-owned state. It makes every candidate and exclusion explainable before
mutation, requires an explicit `--apply`, and gives every collector the same
locking, path-safety, revalidation, and reporting contract.

## 1. Motivation

Orbit currently bounds some state at startup, exposes destructive operations in
individual domains, and leaves other stores unbounded. Those entry points do not
share ownership proofs, retention clocks, concurrency rules, or output. A
top-level family is needed before more collectors are added because an incorrect
cleanup contract can destroy active work, provenance, or audit evidence.

This feature covers only Orbit-owned roots and lifecycle records. Language build
caches are collected only when they reside inside a managed worktree; arbitrary
Cargo, Gradle, Python, or Node caches outside one are not GC candidates.

## 2. Core Concepts

- **Target** — one collector: `worktrees`, `runs`, `logs`, `diagnostics`,
  `audit`, `skills`, `tasks`, or the aggregate `all` target.
- **Plan** — the immutable candidate snapshot built for one invocation. Apply
  consumes that plan but revalidates each item immediately before mutation.
- **Ownership proof** — domain metadata tying a candidate to an Orbit-owned
  root and record. A familiar name or old filesystem mtime is not proof.
- **Retention clock** — the persisted domain event from which age is computed,
  such as a run's terminal transition or a task's terminal history entry.
- **Protection invariant** — a rule that turns uncertainty into a reported skip.
  Protection invariants cannot be disabled by force or retention overrides.
- **Reclaimed** — a completed deletion or lifecycle mutation. In v1, task GC
  archives tasks and does not physically delete task bundles.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Shared grammar, plan/apply/report, and lock | [2_design.md](./2_design.md) | [ORB-10180] |
| Managed worktrees | [2_design.md §3.1](./2_design.md#31-worktrees-workspace) | [ORB-10182] |
| Job-run bundles and records | [2_design.md §3.2](./2_design.md#32-runs-workspace) | [ORB-10183] |
| Operational logs | [2_design.md §3.3](./2_design.md#33-logs-global) | [ORB-10184] |
| Diagnostics partitions | [2_design.md §3.4](./2_design.md#34-diagnostics-workspace) | [ORB-10185] |
| Audit envelopes and blobs | [2_design.md §3.5](./2_design.md#35-audit-workspace-and-global) | [ORB-10186] |
| Generated skills and links | [2_design.md §3.6](./2_design.md#36-skills-global) | [ORB-10187] |
| Terminal task archival | [2_design.md §3.7](./2_design.md#37-tasks-workspace) | [ORB-10188] |
| Aggregate and automatic collection | [2_design.md §9](./2_design.md#9-aggregate-and-automation-policy) | [ORB-10189] |

## Task References

- [ORB-10178] — defined the GC retention and safety contract.
- [ORB-10180] — will implement the shared top-level GC framework.
- [ORB-10182] — will implement managed worktree collection.
- [ORB-10183] — implemented staged terminal run archival and purge.
- [ORB-10184] — will implement operational log collection.
- [ORB-10185] — will implement diagnostics collection.
- [ORB-10186] — will implement audit retention and blob reachability.
- [ORB-10187] — will implement generated-skill collection.
- [ORB-10188] — will implement terminal task archival.
- [ORB-10189] — will implement aggregate and opt-in automatic collection.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
