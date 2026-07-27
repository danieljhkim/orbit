---
title: Orbit Core — Decisions
owner: claude
last_updated: 2026-07-19
status: Accepted
feature: orbit-core
doc_role: decisions
type: design
summary: ADR log for orbit-core crate-boundary decisions, starting with the ORB-10016 orbit-cmd extraction.
tags: [orbit-core, orbit-cmd, architecture, north-star]
paths: ["crates/orbit-core/**", "crates/orbit-cmd/**"]
related_features: [orbit-core]
related_artifacts: [ADR-0203, ADR-0209, ORB-10026]
---

# Orbit Core — Decisions

ADR log for orbit-core's crate boundary. Entries are append-only and ordered
by ascending global ID. **Allocate the global `ADR-NNNN` via `orbit.adr.add`
before writing the heading** — never hand-author a four-digit number. The
store owns ID, status, owner, and links; this file is the long-form narrative
keyed on that same ID. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adr-template-strict) for the full
rules.

## ADR-0203 — Extract the CLI-facing command layer into orbit-cmd

**Status:** Accepted · 2026-07 · [ORB-10016]

**Context.** orbit-core was a ~46k LoC god-crate whose `command/` tree (18
submodules, ~22k LoC with sibling tests) mixed two different kinds of code:
command groups the runtime itself invokes (the `orbit.*` agent tool hosts in
`runtime/orbit_tool_host` and `runtime/v2_host`, the engine hosts in
`runtime/engine`, and bootstrap seeding all call task, learning, docs, search,
semantic, job-run, audit, pipeline, and tool commands as inherent
`OrbitRuntime` methods), and command groups that are pure consumers of the
runtime's public API. The roadmap goal was a `orbit-cmd` crate depending on
orbit-core — never the reverse — but two Rust rules constrain the cut: an
inherent `impl OrbitRuntime` block must live in the crate that defines the
type, and implementing an orbit-engine host trait for `OrbitRuntime` outside
orbit-core would violate the orphan rule.

**Decision.** Move only the command groups the runtime never calls —
workspace doctor, migrate status/dry-run, diagnostics readers, agent-rules
injection, hook install, the learning/review-thread PreToolUse hook, and the
direct v2 activity runner (plus their sibling tests and the `agent-rules.md`
embedded asset) — into a new internal crate `crates/orbit-cmd`. Former inherent
methods become per-module extension traits (`DoctorCommands`,
`MigrateCommands`, `DiagnosticsCommands`, `LearningHookCommands`,
`ActivityV2Commands`;
`orbit_cmd::prelude::*` imports them all), so call sites keep method syntax.
Runtime-entangled groups (task, learning, docs, search, semantic, job, tool,
audit-event, pipeline-run, init/seeding, workflow, skill, activity v1,
policy, executor, backend-resolver, task-migration, review-thread-hook)
remain in `orbit-core::command` as documented residuals. orbit-core exposes
the minimal extra surface orbit-cmd needs — `OrbitRuntime::{paths,
allowlist_known_tool_names, record_event, list_orphaned_running_job_runs}`,
`runtime::{try_resolve_initialized_roots, resolve_global_root,
ResolvedOrbitRoots}`, `config::{validate_layered_config,
resolved_audit_db_path}` (instead of widening the whole `RuntimeConfig`
tree), `command::SYSTEM_AUDIT_IDENTITY`, and pub hook-state helpers on
`command::{learning, review_thread_hook}` — each annotated with an
`[ORB-10016]` comment. Root re-exports in orbit-core's `lib.rs` were trimmed
from ~120 items to the ~60 with a demonstrated import in `orbit-cli`,
`orbit-dashboard`, or `orbit-cmd`; everything else is reachable through its
owning module or crate.

**Rejected alternatives.**

- *Move the whole `command/` tree and invert the hosts behind traits defined
  in orbit-core and implemented in orbit-cmd.* Rejected: the runtime could no
  longer construct itself (consumers would need orbit-cmd to get a working
  tool host), effectively inverting the layering, and the orphan rule blocks
  re-homing the engine-host trait impls anyway.
- *Cyclic dev-dependency (orbit-core dev-depends on orbit-cmd) to move more
  test-only callers.* Rejected: cargo allows it, but it muddies the
  dependency-direction guard and the mental model for a marginal LoC win.
- *Free functions instead of extension traits.* Rejected: every consumer call
  site would change shape (`runtime.doctor_workspace()` →
  `doctor::doctor_workspace(&runtime)`); traits keep the diff mechanical and
  reviewable.

**Consequences.**

- New allowed edges: `orbit-cmd → {orbit-core, orbit-engine, orbit-store,
  orbit-common}`; `orbit-cli → orbit-cmd`; `orbit-dashboard → orbit-cmd`.
  orbit-core must never import orbit-cmd (enforced by
  `scripts/check-dependency-direction.sh`).
- The split is partial by design: orbit-core keeps ~41k LoC because its tool
  hosts *are* the command layer for agents. Shrinking further requires the
  host-inversion redesign this ADR rejects for now, not more file moves.
- Calling a moved command method now requires the extension trait (or
  `orbit_cmd::prelude::*`) in scope; a bare `runtime.doctor_workspace()`
  without the import is a compile error naming the trait.
- Cost: `OrbitRuntime` gained public accessors (`paths()`, `record_event()`,
  `allowlist_known_tool_names()`, …) that exist only for orbit-cmd's benefit —
  the kernel's nominal public API is wider even though the crate is smaller,
  and those items cannot be narrowed again without touching orbit-cmd.

## ADR-0209 — North-star architecture bearing: operations as data behind an operation registry

**Status:** Proposed · 2026-07 · [ORB-10026]

**Context.** Orbit's four consumer surfaces — the CLI (`orbit-cli`), MCP
(`orbit-mcp`), the web dashboard (`orbit-dashboard`), and the in-runtime agent
tool hosts — are four hand-wired adapter layers over the same underlying
operations, so a new operation is plumbed by hand up to four times. The same
structural shape keeps constraining refactors: inherent `impl OrbitRuntime`
methods plus the orphan rule forced the [ORB-10016] / ADR-0203 orbit-cmd
extraction to leave the runtime-entangled command groups behind in orbit-core
as documented residuals, and the same wall shelved the docs+search
pluginization (`docs/design/orbit-docs-plugin/1_scope.md` — pending commit).
Point refactors treat symptoms; what was missing is a recorded long-term
bearing that future refactors steer by.

**Decision.** Record five bearings as orbit's north star. This is an
**incremental bearing, not a rewrite mandate** — no code change is required by
this ADR, and existing code is not wrong for predating it.

1. **Operations as data, not inherent methods.** Every orbit operation is
   eventually a serializable request/response pair with a handler registered
   in an operation table; the four surfaces become derived adapters over that
   registry, and the inherent-impl/orphan-rule constraint dissolves because
   handlers are registry entries, not inherent methods on `OrbitRuntime`.
2. **Knowledge/execution split.** Orbit is two products — a knowledge store
   (tasks, learnings, ADRs, docs) and an execution engine (activities, jobs,
   agent providers) — glued by one runtime. Bearing: two systems sharing only
   a kernel (IDs, errors, audit), mirroring the constellation split
   (polaris = knowledge, worker = execution).
3. **Events over side-effects.** The task-mutation → semantic-index coupling
   becomes a transactional SQLite outbox consumed by the indexer, replacing
   the lossy in-process `EmbedWorker` enqueue (best-effort batches, drops on
   queue-full, debug-level failure logging).
4. **One retrieval trait, two backends.** orbit-search (workspace-local) and
   sextant (constellation-wide) become deployment choices behind one retrieval
   interface, dissolving the two-stack question.
5. **Crates follow build boundaries, not taxonomy.** Crate splits are
   justified by compile-graph and dependency-direction needs, not conceptual
   category. Explicitly kept: the ADR-005 companion-subprocess packaging
   pattern ([orbit-search 4_decisions.md ADR-005](../orbit-search/4_decisions.md),
   global ADR-0117), the YAML+SQLite layered store in orbit-store, and the
   stability-tier markers (ARCHITECTURE.md §Stability tiers).

Adoption is opportunistic: when a surface is added or a command group is
touched for other reasons, move that slice to request/response + registry
then. Future ADRs cite this bearing when steering by it, or supersede it if
the bearing itself changes.

**Rejected alternatives.**

- *Status quo as the implicit bearing.* Keeps charging up to 4× adapter wiring
  per operation and guarantees the next boundary refactor hits the same
  inherent-impl/orphan-rule wall with no recorded direction.
- *Big-bang registry rewrite.* Months of churn across every surface with no
  incremental payoff and high regression risk in a codebase that ships
  continuously; rejected in favor of the touch-it-move-it model.

**Consequences.**

- Future architecture ADRs have a fixed point to cite or supersede.
- Slices touched after this ADR should trend toward request/response +
  registry; reviewers can ask "why not the registry shape?" when a new
  hand-wired adapter appears.
- The knowledge/execution split gives crate and module moves a destination:
  kernel-shared code is IDs/errors/audit only.
- No single code anchor; convention enforced via review.
- Cost: two coexisting idioms during the (unbounded) incremental transition —
  readers must recognize both the registry shape and the legacy
  inherent-method shape — and request/response indirection adds per-operation
  boilerplate (request type, response type, registration) compared to calling
  an inherent method directly.

## Task References

- [ORB-10016] — extracted `orbit-cmd` from orbit-core, converted moved command groups to extension traits, and trimmed root re-exports.
- [ORB-10026] — authored the ADR-0209 north-star operation-registry architecture bearing.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
