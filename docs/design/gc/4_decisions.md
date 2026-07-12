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
related_artifacts: [ADR-0220, ORB-10178, ORB-10180]
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

## Task References

- [ORB-10178] — selected and specified the shared GC contract.
- [ORB-10180] — implemented the shared framework and top-level command grammar.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
