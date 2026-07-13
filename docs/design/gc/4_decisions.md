---
title: Garbage Collection — Decisions
owner: codex
last_updated: 2026-07-13
status: Draft
feature: gc
doc_role: decisions
type: design
summary: Records the durable choice of one explicit, safety-first Orbit garbage-collection family.
tags: [gc, retention, safety]
paths: ["docs/design/gc/**", "crates/orbit-cli/src/command/gc/**", "crates/orbit-core/src/command/gc/**"]
related_features: [gc]
related_artifacts: [ADR-0220, ADR-0221, ORB-10178, ORB-10180, ORB-10184, ORB-10186]
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
- Audit collection implements the same contract by deleting expired envelopes
  before sweeping blobs and recomputing reachability at blob revalidation;
  holds, exports, and retained job-run bundles participate in the mark set. Its
  domain writer protocol is a workspace audit writer/GC guard (ORB-10186): an
  advisory lock every v2/loop/blob publication path shares with the collector,
  held under the host GC lock from the final mark/fingerprint validation through
  the envelope/blob unlink so a concurrent writer can neither strand a retained
  reference nor lose an append. Because a blob and the reference that names it
  are published by two separate guarded calls, a durable per-blob
  pending-publication marker (`state/audit/pending/<hash>`) bridges the gap: the
  collector treats a fresh marker as a live reference (fail closed) and reclaims
  only markers whose retention window has closed, so no published reference can
  point at a swept blob and a never-published blob leaks no longer than the
  retention window.
- Code anchors: `execute_gc` freezes and consumes plans under the host lock;
  `validate_candidate_path` enforces containment and no-follow path checks.
  Both cite ADR-0220 at the enforcement point; collector review covers the
  remaining domain-specific invariants.
- Cost: All apply passes serialize behind one host-wide lock and pay
  per-candidate revalidation, and workspace owners must repeat any global GC
  settings they want in a complete workspace policy instead of inheriting a
  merged fragment.

## ADR-0221 — Startup log rotation is non-destructive; deletion goes through explicit GC

**Status:** Accepted · 2026-07 · [ORB-10184]

**Context.** ADR-0220 makes explicit `--apply` the single mutation gate for
Orbit GC: the shared host lock, candidate revalidation, deletion manifest, and
error report all hang off it. An earlier revision of this ADR kept opportunistic
subscriber-init *deletion* of log archives (routed through the shared
`plan_prune` classifier) to preserve the ORB-00415 disk bound until automated
`orbit gc logs` (ORB-10189) lands. But that startup delete pass performed raw
best-effort `remove_file` calls *outside* the host lock, revalidation, manifest,
and `--apply` gate — a second, weaker retention mutation path that contradicts
the ADR-0220 contract. The alternatives were to keep that branch-local exception
or to make startup non-destructive and defer all deletion to explicit apply.

**Decision.** Startup is **non-destructive**. Subscriber-init and the sweep hook
roll an oversized active file (rename — non-destructive) and only *report* the
archives that `orbit gc logs --apply` would reclaim (`rotate_and_report` /
`report_prunable`), computed via the shared `plan_prune` classifier. No startup
path unlinks an archive. Every archive deletion goes through
`orbit gc logs --apply` and its ADR-0220 machinery (host lock, revalidation,
manifest, report). The active inode is never deleted or truncated on any path
(§5 invariant 3).

**Consequences.**

- One mutation gate, no bypass: the subscriber-init hook can no longer delete
  outside the ADR-0220 contract, so the parent safety contract is not weakened.
- One retention policy still: startup reporting and `orbit gc logs` share
  `plan_prune`, so the plan the operator sees at startup matches exactly what
  apply reclaims.
- Trade-off: until automated `orbit gc logs` lands (ORB-10189), archives
  accumulate between explicit `orbit gc logs --apply` runs rather than being
  trimmed at every startup. The active file stays bounded by rotation; total
  archive bytes are bounded only when GC is run. This is the accepted cost of
  routing all deletion through the explicit gate.
- §3.3/§9/§10 describe non-destructive startup (rotate + report) rather than
  startup deletion; the §5 active-inode invariant is unchanged.

## Task References

- [ORB-10178] — selected and specified the shared GC contract.
- [ORB-10180] — implemented the shared framework and top-level command grammar.
- [ORB-10184] — implemented log GC; made startup rotation non-destructive with
  all deletion behind the explicit apply gate (ADR-0221).
- [ORB-10186] — implemented unified audit retention and blob mark-and-sweep.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
