---
title: Garbage Collection — Decisions
owner: codex
last_updated: 2026-07-12
status: Draft
feature: gc
doc_role: decisions
type: design
summary: Records the durable choice of one explicit, safety-first Orbit garbage-collection family.
tags: [gc, retention, safety]
paths: ["docs/design/gc/**", "crates/orbit-cli/src/command/gc/**", "crates/orbit-core/src/command/gc/**"]
related_features: [gc]
related_artifacts: [ADR-0220, ADR-0221, ORB-10178, ORB-10180, ORB-10184]
---

# Garbage Collection — Decisions

ADR log for garbage collection. Entries are append-only and ordered by ascending
global ID. The store owns ID, status, owner, and links; this file is the
long-form narrative keyed on the same allocated ID. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full rules.

## ADR-0220 — One safety contract for explicit Orbit garbage collection

**Status:** Accepted · 2026-07 · [ORB-10178]

**Context.** Orbit cleanup is split between domain commands and startup hooks,
while a top-level collector must coordinate global and workspace state without
racing live owners. Real alternatives were to keep domain-specific mutation
surfaces, put worktree cleanup under `orbit run`, let each collector define its
own lock/report/force rules, or make bare invocation destructive.

**Decision.** Adopt one top-level `orbit gc [TARGET]` family whose default is a
non-mutating plan and whose only mutation gate is explicit `--apply`. Every
target uses a shared immutable plan/report contract, one host-wide apply lock
plus domain-specific atomic revalidation, source-of-truth retention clocks, and
non-bypassable containment, symlink, current-owner, and ambiguity protections;
`all` composes the same collectors.

**Consequences.**

- Existing cleanup entry points delegate to shared collectors or become
  non-mutating compatibility shims; `orbit run gc` is rejected because
  retention is storage lifecycle, not workflow execution.
- Global targets take policy only from global config. Workspace targets use the
  existing complete-document workspace-replaces-global rule, with CLI
  overrides highest.
- Partial failure preserves successful mutations, reports every skip/error, and
  returns non-zero; reruns are idempotent.
- Code anchors: `execute_gc` freezes and consumes plans under the host lock;
  `validate_candidate_path` enforces containment and no-follow path checks.
  Both cite ADR-0220 at the enforcement point; collector review covers the
  remaining domain-specific invariants.
- Cost: All apply passes serialize behind one host-wide lock and pay
  per-candidate revalidation, and workspace owners must repeat any global GC
  settings they want in a complete workspace policy instead of inheriting a
  merged fragment.

## ADR-0221 — Startup log pruning retained through the shared GC classifier

**Status:** Accepted · 2026-07 · [ORB-10184]

**Context.** The design (§9–§10) aspires to have subscriber-init hooks stop
deleting log archives once `orbit gc logs` owns retention, deferring all
deletion to explicit apply. But Orbit has no resident daemon and automated
`orbit gc logs` is a future task (ORB-10189); removing opportunistic startup
pruning now would regress the ORB-00415 disk bound on always-on hosts until that
automation lands. The alternative was rotate-only startup with archives
accumulating until an operator runs `orbit gc logs --apply`.

**Decision.** Keep opportunistic startup pruning, but route both it and
`orbit gc logs` through one extracted classifier
(`log_rotation::plan_prune`). Startup deletes best-effort as before; the CLI
collector plans/applies the same age + total-size policy with reporting,
locking, and revalidation. Neither path ever deletes or truncates the active
inode (§5 invariant 3).

**Consequences.**

- One retention policy: startup pruning and `orbit gc logs` cannot disagree
  because they share `plan_prune`; the CLI adds an inspectable, on-demand
  surface over the same budgets.
- v1 log GC does not require a scheduler to keep archives bounded; ORB-00415's
  disk safety is preserved.
- §3.3/§9/§10 are updated to describe shared-classifier pruning rather than
  "startup hooks do not delete"; the §5 active-inode invariant is unchanged.
- Cost: startup still performs a small delete pass on every subscriber init;
  when automated `orbit gc logs` lands (ORB-10189) this should be revisited so
  deletion can move fully behind the explicit apply gate.

## Task References

- [ORB-10178] — selected and specified the shared GC contract.
- [ORB-10180] — implemented the shared framework and top-level command grammar.
- [ORB-10184] — implemented log GC and retained startup pruning via the shared
  classifier (ADR-0221).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
