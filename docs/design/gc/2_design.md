---
title: Garbage Collection — Design
owner: codex
last_updated: 2026-07-13
status: Draft
feature: gc
doc_role: design
type: design
summary: Specifies GC grammar, collector ownership, retention clocks, safety invariants, locking, and reports.
tags: [gc, retention, safety]
paths: ["crates/orbit-cli/src/command/gc.rs", "crates/orbit-core/src/command/gc.rs", "crates/orbit-core/src/command/gc_logs.rs", "crates/orbit-common/src/utility/log_rotation.rs", "crates/orbit-core/src/config/**"]
related_features: [gc, activity-job, auditability, task-artifacts, worktree-artifacts]
related_artifacts: [ORB-10178, ORB-10180, ORB-10184, ADR-0220, ADR-0221]
---

# Garbage Collection — Design

This document is the normative v1 contract for planning and applying retention
to Orbit-owned global and workspace state. Collector implementation details may
vary, but they may not weaken the mutation gate, ownership rules, protection
invariants, revalidation, lock, or report semantics defined here.

The shared protocol is implemented in
`crates/orbit-core/src/command/gc.rs`; the command grammar and report rendering
live in `crates/orbit-cli/src/command/gc.rs`. Domain collectors plug into that
protocol in the dependent tasks listed below.

## 1. Command Grammar and Mutation Gate

The family is:

```text
orbit gc [TARGET] [--workspace <id-or-path> | --global] [--retention <duration>]
         [--json] [--apply]

TARGET := worktrees | runs | logs | diagnostics | audit | skills | tasks | all
```

The omitted target means an overview of all targets available in the resolved
scope. `orbit gc`, `orbit gc all`, and every individual target are plan-only by
default. They scan and report but perform no deletion, archival, cancellation,
rotation, link repair, or other lifecycle mutation. `--apply` is required for
every mutation, including when the command is called by compatibility shims or
automation. There is no interactive confirmation that silently substitutes for
`--apply`.

`--retention` is a target-appropriate one-invocation override. It affects
eligibility only; it cannot waive a protection invariant. Collectors with more
than one retention class expose explicit qualified overrides rather than giving
one ambiguous duration multiple meanings. Unsupported scope/target combinations
are reported as `not_applicable`; they never fall through to another root.

`--workspace` selects one registered workspace without changing process cwd.
When neither scope flag is present, Orbit uses the discovered current workspace
for workspace targets and the global root for global targets. Outside a
workspace, workspace targets report that selection is required. `--global`
selects only global targets; it is not shorthand for every registered workspace.
Cross-workspace collection is reserved for a separately explicit aggregate
operator surface.

## 2. Shared Plan and Apply Protocol

Every collector implements the same phases:

1. **Resolve** the target, owned roots, effective configuration, clock, and
   read-only domain stores.
2. **Scan** entries without following symlinks. Record every recognized item,
   unknown item, and scan error.
3. **Classify** from persisted domain state into eligible or a stable skip code.
4. **Freeze** an immutable plan containing candidate identity, ownership proof,
   retention evidence, expected state/version, path identity, and byte estimate.
5. **Apply**, only when explicitly requested, by revalidating and mutating each
   frozen candidate under the locks in §6.
6. **Report** the same plan and outcomes in human or JSON form.

An apply invocation acquires the GC lock before scanning and holds it while it
builds and consumes its one immutable plan. A plan-only invocation does not
persist authority for a later apply: a later `--apply` invocation builds a new
plan because clocks and owners may have changed. Given the same roots, config,
clock, and store snapshot, plan-only and apply select the same candidates;
apply may only reduce mutations by turning a candidate into a revalidation skip
or error. It may never add an item that was absent from the frozen plan.

Planning has no side effects, including no opportunistic store migration,
rotation, stale-owner cancellation, index rewrite, or lock cleanup. Opening a
store for planning must use a read-only/no-migration path.

## 3. Targets, Ownership, and Retention Clocks

Age is computed from the named persisted domain timestamp. Filesystem mtime is
never the sole eligibility clock. If the authoritative timestamp or ownership
proof is absent, malformed, or contradictory, the item is skipped as ambiguous.

### 3.1 Worktrees (workspace)

The collector owns only worktrees registered by Orbit beneath the resolved
workspace worktree root. Eligibility uses the owning run's persisted terminal
transition: success/cancelled and failed/interrupted classes have separate
retention. Pending, running, resumable, live-owned, current, dirty source-bearing,
unknown-owner, unmerged, unpushed, or liveness-inconclusive worktrees are
protected. Unknown directories are inventory entries, not candidates. Removal
uses Git worktree operations and is language-neutral. On-terminal cleanup calls
the same classifier and apply primitive as manual GC.

### 3.2 Runs (workspace)

The run collector coordinates authoritative rows, steps, reservations,
checkpoints/scoreboards, task references, and legacy/regenerable bundles. Its
clock is the persisted terminal transition. Archive and purge are distinct
stages with distinct ages, and failed/interrupted evidence may have a longer
retention. Active, resumable, task-held, or liveness-inconclusive runs are
protected. Audit envelopes and blobs are not run-owned deletion side effects;
they remain the audit collector's responsibility.

### 3.3 Logs (global)

The log collector owns Orbit-created operational log archives beneath the
global state root: the JSONL tracing feed (`state/logs/orbit.jsonl`, overridable
via `ORBIT_LOG_PATH`) and the macOS sweep log (`logs/sweep.log`). Only dated
`<active>.<stamp>` archives are candidates; the active inode is never one, so a
writer holding it open is unaffected. The existing age and total-byte budgets
share one classifier — `log_rotation::plan_prune` — with non-destructive startup
rotation (ADR-0221), so the plan the CLI surface applies matches exactly what
subscriber-init reporting observes. Reports distinguish age-selected from
size-selected files via the item `action` (`delete-age` / `delete-size`) and
record reclaimed bytes and per-file errors. An explicitly configured
`ORBIT_LOG_PATH` is an owned active log even when it resolves outside the default
scope root: its parent directory is surfaced as an allowlisted owned root (the
collector's `owned_roots`), so its archives are planned and reclaimed while every
deletion still passes the canonical no-follow containment gate — it is honored,
not skipped. Journald, system logs, and third-party logs are out of scope.
[ORB-10184]

### 3.4 Diagnostics (workspace)

This collector owns closed metrics and diagnostic-friction JSONL partitions
under the workspace diagnostics root. Its clock is the partition boundary plus
persisted record/closure timestamps. The current partition, writer-owned files,
malformed partitions, canonical friction records, tasks, learnings, and audit
evidence are protected.

### 3.5 Audit (workspace and global)

Audit collection covers the legacy event store in its owning scope and
workspace v2 events, loop envelopes, and content-addressed blobs. Event age uses
the persisted event timestamp. Blob deletion is never age-based: a mark phase
walks all retained envelopes, holds, exports, and retained-run references, and
only unreachable blobs enter the sweep plan. Ordering must ensure a retained
envelope never points at a deleted blob. The GC operation writes a deletion
manifest or out-of-band audit event that the active plan cannot recursively
collect.

### 3.6 Skills (global)

Skill collection covers generated directories and supported agent-root links
whose Orbit ownership is proven by manifest/hash and retirement metadata. Age,
when relevant, uses a persisted retirement timestamp. Name matching, a broken
link, or mtime is insufficient. Modified generated content, same-named user
content, links targeting elsewhere, plugin caches, and third-party content are
protected. Only the link itself may be unlinked; a symlink target is never
traversed for deletion.

### 3.7 Tasks (workspace)

V1 task collection is reversible archival, not physical bundle deletion. It
uses the persisted transition into an eligible terminal status. Active states,
open review threads, keep/exemption metadata, unresolved relations, and other
lifecycle holds are protected. Apply delegates to the ordinary task lifecycle
mutation so history, audit, projections, relations, and search indexes remain
consistent. A future purge requires a separate export/restore, tombstone, and
referential-integrity decision.

### 3.8 All

`all` contains each target available in the explicitly resolved scope and uses
the exact individual collectors. It freezes one aggregate plan. Apply order is
stable and dependency-aware: `worktrees`, `runs`, `diagnostics`, `skills`,
`tasks`, `audit`, then `logs`. This preserves worktree ownership evidence before
run cleanup and lets audit reachability observe the post-run/task retained set.
Independent later collectors may continue after an error; a collector whose
prerequisite failed is reported as dependency-blocked.

## 4. Configuration and Precedence

GC uses typed `[gc]` configuration with per-target subsections and conservative
built-in defaults. Exact key names and defaults land with their collector, but
every key identifies its unit and retention class; zero means immediate
eligibility only where the key explicitly permits it. Invalid values fail plan
construction before mutation.

Precedence is:

1. explicit CLI override for this invocation;
2. the one effective config document for the target scope;
3. built-in target defaults.

Global targets read only the global config. A workspace target reads the
workspace config when it exists; that document replaces, rather than merges,
global config. Therefore a workspace config is a complete policy document:
missing GC keys fall back to built-in defaults, not values from global config.
If no workspace config exists, the global document is the workspace target's
effective document. Reports identify `config_source` and resolved values.

For `all`, each subplan resolves independently: global subtargets use the global
document, while workspace subtargets use the complete workspace policy above.
A workspace cannot use its config to broaden global roots or weaken global
protection rules.

## 5. Non-bypassable Safety Invariants

The following are eligibility requirements, not warnings. No `--force`, short
retention, environment variable, compatibility command, or automation setting
may bypass them.

1. **Root containment.** Each path is derived from an allowlisted `WorkspacePaths`
   or global-root field. Lexical normalization must reject parent traversal, and
   resolved identity must remain beneath the already validated owned root.
   Arbitrary user-supplied deletion roots are unsupported.
2. **Symlink safety.** Scan with `symlink_metadata`; do not follow symlinks while
   measuring or deleting. A proven Orbit-owned symlink may be unlinked as an
   object, but its target is never recursively removed. A symlink/reparse point
   in any unexpected path component makes the candidate ambiguous.
3. **Current-process protection.** Never mutate the current working worktree,
   the running executable or its containing installation, the active log inode,
   this process's run/reservation, or state locked by this process. Ancestor and
   identity comparisons use resolved paths/file identity, not string spelling.
4. **Live-owner protection.** A live PID/lease/run claim/writer or a resumable
   owner prevents mutation. PID reuse must be ruled out with a persisted owner
   token/start identity. Permission errors and inconclusive liveness are skips,
   never evidence that an owner is dead.
5. **Ownership and ambiguity.** Every mutation needs positive domain ownership
   and a source-of-truth retention clock. Unknown, malformed, dirty, conflicting,
   or partially readable items remain in place with a reason.
6. **Atomic revalidation.** Immediately before mutation, re-read ownership,
   state/version, retention evidence, liveness, path identity, and holds under
   the GC lock plus the domain writer lock or store transaction. Use
   descriptor-relative/no-follow mutation where available. If the platform
   cannot close a path race safely, skip rather than falling back to an unsafe
   recursive delete.
7. **Branch and evidence preservation.** Collection never deletes an unmerged
   or unpushed task branch, reachable audit blob, held export, or evidence still
   required by a retained run/task.

## 6. Mutual Exclusion and Writer Coordination

Every apply entry point, individual or aggregate, acquires one host-wide
exclusive lock beneath the global Orbit state root. This serializes global and
workspace GC so two workspaces cannot concurrently touch shared logs, skills,
or stores. Lock metadata contains process identity, start identity, command,
scope, and acquisition time. Acquisition has a bounded wait and returns a clear
non-mutating error for a live holder; stale-lock recovery itself requires a
conclusive dead-owner check.

The GC lock coordinates collectors, not ordinary writers. Before each mutation,
the collector also participates in the domain's writer protocol: a database
transaction/CAS, run-claim or task lock, log rotation lock, or equivalent.
Lock order is always GC lock, then domain lock, then filesystem operation.
Collectors never wait for a domain lock while holding locks in the reverse
order. Plan-only scans may run concurrently and must tolerate changes as
reported uncertainty.

## 7. Standard Report Contract

Human and JSON modes are projections of one in-memory `GcReport`; they contain
the same targets, scopes, counts, byte estimates, skips, errors, and final
outcome. Human mode may abbreviate item detail on screen but must state where
the complete deletion manifest was written. JSON field meanings are versioned:

```json
{
  "schema_version": 1,
  "mode": "plan|apply",
  "plan_id": "stable invocation identifier",
  "scope": {"kind": "global|workspace", "workspace_id": null, "root": "..."},
  "config_source": "builtin|global|workspace",
  "started_at": "RFC3339",
  "finished_at": "RFC3339",
  "outcome": "clean|partial|failed",
  "targets": [{
    "target": "worktrees",
    "counts": {"scanned": 0, "eligible": 0, "reclaimed": 0},
    "bytes": {"scanned": 0, "eligible": 0, "reclaimed": 0, "estimate_complete": true},
    "items": [{"id": "...", "action": "delete|archive|unlink|rotate", "status": "eligible|reclaimed|skipped|error", "bytes": 0}],
    "skipped": [{"id": "...", "code": "live_owner", "reason": "..."}],
    "errors": [{"id": "...", "phase": "scan|revalidate|apply", "code": "...", "message": "..."}]
  }]
}
```

Counts are item counts, never inferred from array truncation. `scanned` includes
recognized, unknown, skipped, and errored entries encountered in the owned
scope. `eligible` is the frozen plan count. `reclaimed` counts only completed
mutations; in plan mode it is zero. Bytes use saturating `u64` estimates;
unknown sizes do not become zero silently and set `estimate_complete=false`.
Reclaimed bytes are measured or best-effort verified after mutation and may be
lower than eligible bytes. Skip and error codes are stable machine values;
reasons/messages are redacted human explanations.

A clean plan or apply exits zero. Any scan/apply error or dependency-blocked
target makes the outcome partial/failed and the process non-zero, while ordinary
policy skips such as `retained`, `live_owner`, or `not_applicable` remain a
successful report. Lock contention is a non-mutating error. JSON is written as
one valid document to stdout; diagnostics do not corrupt it.

## 8. Partial Failure, Audit, and Recovery

Candidates are independent unless a collector declares a transaction group.
An item failure is recorded and later independent items continue. Completed
mutations are not rolled back by recreating deleted state. A transactional
collector must order mutations so interruption leaves either the old state or a
restart-safe intermediate state; re-running plans what remains and is
idempotent. Aggregate collection continues independent targets but reports
targets blocked by a failed prerequisite.

Every completed mutation is represented in an append-only deletion manifest
containing the plan ID, target, candidate identity, ownership/clock evidence,
action, bytes, and result. Where safe, Orbit also emits a normal audit event.
The manifest/event for the active pass is excluded from its own candidate
snapshot, preventing recursive audit/log growth or self-deletion.

## 9. Aggregate and Automation Policy

`orbit gc all` uses one host-wide lock and aggregate snapshot, the stable order
in §3.8, and configurable time/item/reclaimed-byte budgets. A budget is checked
before starting the next item; reaching it is a reported bounded stop, not an
error and never interrupts an item mid-transaction.

Automatic collection is disabled by default. Opt-in automation must name its
scope and targets, set the collector's explicit apply argument, and use the same
budgets, lock, collectors, protections, and reports as an operator invocation.
It may be a scheduled routine, but it cannot discover, create, or dispatch
tasks. Startup hooks perform no lifecycle mutation beyond non-destructive
active-file rotation; they never delete an archive and never touch the active
inode. They only *report* — via the shared classifier
(`log_rotation::plan_prune`) — the archives that `orbit gc logs --apply` would
reclaim. Per ADR-0221 all archive deletion is owned exclusively by the explicit
`orbit gc logs --apply` gate; the future automated log GC (ORB-10189) will drive
that same gate on a schedule.

## 10. Compatibility and Surface Ownership

- **`orbit audit prune`.** Keep a deprecation shim for one documented migration
  window. The old bare destructive form becomes plan-only; mutation requires a
  new explicit `--apply`. `--older-than` maps to the legacy-event retention
  override, and both paths call the audit collector. There is no legacy bypass
  around blob reachability, holds, locking, reporting, or revalidation.
- **Log startup rotation.** The retention classifier is extracted as
  `log_rotation::plan_prune`. Startup continues non-destructive active-file
  rotation and only *reports* reclaimable archives through that shared
  classifier (`rotate_and_report`); it never unlinks an archive. `orbit gc logs`
  plans/applies the identical age + total-size policy with reporting, locking,
  and revalidation, and deletion requires `--apply` (there is no config-only
  substitute). Per ADR-0221 all archive deletion is behind that explicit gate,
  keeping the ADR-0220 single-mutation-gate contract intact; automated log GC
  (ORB-10189) will drive the same gate on a schedule.
- **Skill unlink/init cleanup.** Explicit `orbit skill unlink` remains an
  operator-owned uninstall action, but generated-content retirement and init
  cleanup delegate to the skill ownership classifier. Existing `force` flags
  may replace an installation during init but cannot weaken GC ownership,
  symlink, or containment invariants.
- **Rejected `orbit run gc`.** No compatibility alias is added. `run` owns
  workflow/job execution; retention spans stores unrelated to a run. Any
  unreleased or experimental `orbit run gc` implementation is removed or emits
  a non-mutating diagnostic pointing to `orbit gc worktrees`; it must never
  preserve old destructive semantics.

## 11. Concerns & Honest Limitations

- Portable descriptor-relative deletion and reliable open-file ownership differ
  by platform. Unsupported proofs reduce reclamation by producing skips; they do
  not justify weaker deletion.
- Byte totals are estimates for sparse files, reflinks, compressed stores, and
  concurrently changing trees. Reports expose incompleteness instead of
  claiming exact disk blocks.
- A host-wide lock intentionally limits GC throughput. This is preferable to
  cross-workspace races while collectors share global roots; finer locking
  requires a future proof that preserves the same safety.
- V1 does not physically purge tasks, collect arbitrary build caches, repair
  ambiguous ownership, or promise that every skip can be resolved automatically.
- This document defines the contract before collectors exist. Each implementing
  task must add fake-clock, temp-root, race, symlink, partial-failure, and
  idempotency tests appropriate to its domain.

## Task References

- [ORB-10178] — defined this retention and safety contract.
- [ORB-10180] — will implement the shared GC framework.
- [ORB-10182] — will implement managed worktree collection.
- [ORB-10183] — will implement run retention.
- [ORB-10184] — unified log retention: `orbit gc logs` + shared `plan_prune` (ADR-0221).
- [ORB-10185] — will implement diagnostics retention.
- [ORB-10186] — will implement audit and blob collection.
- [ORB-10187] — will implement generated-skill collection.
- [ORB-10188] — will implement task archival.
- [ORB-10189] — will implement aggregate and automatic collection.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
