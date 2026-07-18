---
title: Host Registry — Decisions
owner: claude
last_updated: 2026-07-18
status: Accepted
feature: host-registry
doc_role: decisions
type: design
summary: Accepted ADR log for the coupled Host Registry and MCP Bridge v1 contract.
tags: [host-registry, mcp-bridge, multi-host, placement]
paths: ["crates/orbit-registry/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge]
related_artifacts: [ORB-00424, ORB-10245, ORB-10248, ORB-10249, ORB-10255, ORB-10257, ORB-10267, ORB-10258, ORB-10268, ORB-10302, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235]
---

# Host Registry — Decisions

ADR log for `host-registry`. Entries are append-only and ordered by global ID.
The Orbit ADR store owns their allocation, status, and task link; this document is
the long-form feature log. The first seven entries are the consolidated v1
behavior contract shared with [mcp-bridge](../mcp-bridge/4_decisions.md);
ADR-0235 records the implementation boundary that keeps that contract singular.

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Accepted · 2026-07 · [ORB-10245] accepted the coupled v1 contract; [ORB-10248] implemented the workspace boundary; [ORB-10249] applied it to task coordination.

### Context

Cross-machine work needs a coordination authority, a knowledge author, and an
execution destination; making them one authority would scatter task state or make
ownership implicit.

### Decision

Use exactly one coordination hub for every workspace, declare one workspace owner,
and select execution placement per run with the owner as the default. Persist the
logical workspace/owner separately from machine-local checkout bindings.

### Consequences

- Coordination writes remain hub-routed while knowledge authorship remains owner-bound.
- Logical workspace lookup never requires or fabricates a checkout path; local path lookup consults checkout bindings only.
- Task coordination, global relation resolution, and dependency readiness use logical workspace IDs and never require either workspace checkout.
- Cost: hub downtime stalls coordination for every workspace, and disconnected machines cannot write coordination records.

## ADR-0227 — Stable machine identity, registry, and out-of-band hub pin

**Status:** Accepted · 2026-07 · [ORB-10245] froze the host identity boundary; [ORB-10255] implemented the durable registry core; [ORB-10267] added operator administration (register/list/rename/retire), current-machine `host.toml` rename coordination, and the hub-global `registry_revision`; [ORB-10268] enforced the machine-global trust and exact hub/store pin at the MCP boundary.

### Context

Hostname-derived strings and per-workspace transport targets can silently redirect a
machine or elevate repository configuration.

### Decision

Give each machine an immutable generated `machine_id`, keep the registry at the hub,
and pin the one hub `machine_id` out of band in machine-local `mcp.toml`.

### Consequences

- Names resolve at binding time and persisted records retain stable identity.
- Current and historical names remain reserved across active and retired lifecycle
  states; explicit rename appends a permanent alias and registration cannot rename
  or reactivate.
- Cost: bootstrap transfers hub identity out of band, and registry/trust drift needs explicit diagnosis.

## ADR-0228 — Local placement broker with capability-set filtering

**Status:** Accepted · 2026-07 · [ORB-10245] froze tool routing and authorization; [ORB-10267] added the `orbit.host.list` and `orbit.workspace.list` canonical discovery tools (hub placement, operator capability, workspace-unscoped) reading one sanitized path-free registry snapshot; [ORB-10268] implemented the fixed checkoutless hub endpoint and exact scalar-capability surface.

### Context

One remote MCP target must preserve local graph and documentation behavior without
equating where a tool runs with who may invoke it.

### Decision

Use a local placement broker. Every exposed canonical tool has exactly one of
`hub`, `owner`, `local-derived`, or `composite` placement and an independently
filtered non-empty capability set.

### Consequences

- Conformance records placement and allowed capabilities for every exposed tool.
- Cost: the broker owns route preflight, composite audit, and capability-by-placement coverage.

## ADR-0229 — Owner-authored knowledge with hub-global IDs and explicit replicas

**Status:** Accepted · 2026-07 · [ORB-10245] fixed the one-writer knowledge rule.

### Context

Knowledge needs global IDs without making a hub checkout or a stale replica a second
author.

### Decision

The hub allocates global IDs, the declared owner authors current knowledge, and Git
replicas are opt-in reads marked as replicas. The hub never proxies to a spoke owner.

### Consequences

- A non-owner agent routes actionable work as a task to the owner.
- Cost: finalize failure consumes a valid unused ID, and current spoke-owned knowledge is unavailable off-owner.

## ADR-0230 — Pull-based leases with immutable placement and explicit recovery

**Status:** Accepted · 2026-07 · [ORB-10245] fixed runner delivery semantics.

### Context

A hub-push executor model needs outbound spoke routes and obscures the placement
selected and leased for a run.

### Decision

Spokes poll the hub for placed runs. Requested and actual placement are immutable;
pre-start loss permits redelivery, while post-start uncertainty is
`recovery_required` and needs explicit recovery.

### Consequences

- The hub is a mailbox and never opens a route to a spoke.
- Cost: pickup latency follows poll cadence and an interrupted started run is not silently reassigned.

## ADR-0231 — Committed-routine ownership with host-local cursors

**Status:** Accepted · 2026-07 · [ORB-10245] fixed routine execution ownership;
[ORB-10270] supplied the registry/cache validation and reassignment evidence.

### Context

Git-committed routines converge to many checkouts, while scheduler cursor and pause
state must remain local to the executing host.

### Decision

A committed routine is owned by its registry-validated host pin; unpinned committed
routines fail closed, and each host retains its own cursor and pause state.

### Consequences

- Reassignment is a reviewed pin change rather than a git-status inference.
- Cost: handoff starts with no migrated cursor and existing committed routines need explicit pins.

## ADR-0232 — Retire Bridge’s Orbit-shaped contract

**Status:** Accepted · 2026-07 · [ORB-10245] set the cutover boundary.

### Context

Bridge parity duplicates Orbit schemas, errors, and workflow declarations even
though Orbit is the canonical domain owner.

### Decision

Retire Bridge’s Orbit-shaped contract after Orbit MCP reaches parity; Bridge remains
for its non-Orbit constellation domains.

### Consequences

- Clients register Orbit and Bridge side by side during migration.
- Cost: cutover temporarily maintains two registrations and requires deletion of a compatibility layer.

## ADR-0235 — Make orbit-registry the singular host/workspace registry domain crate

**Status:** Accepted · 2026-07 · [ORB-10302] established the crate boundary.

### Context

C3 left identity, workspace-catalog, cache, and registry-service behavior in
`orbit-core`, while `orbit-registry` exposed an unused opaque-byte replication and
merge model that contradicted the singular-hub contract.

### Decision

Repurpose `orbit-registry` as the machine/workspace registry domain. It owns host
identity, local catalog/checkouts/roles, cache semantics, and the store-backed
service. Keep runtime profile/ship construction in `orbit-core`, persistence in
`orbit-store`, shared DTOs in `orbit-common`, and reject any
`orbit-store -> orbit-registry` edge.

### Consequences

- The dependency direction is `orbit-core -> orbit-registry -> orbit-store -> orbit-common`, with the existing direct `orbit-core -> orbit-store` edge retained.
- Temporary `orbit-core` compatibility re-exports avoid an atomic caller rewrite without retaining duplicate implementations.
- Cost: `orbit-registry` is no longer a consumer-agnostic leaf and now compiles the store layer; reversing the boundary requires another domain move or a cycle-prone abstraction.

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10248] — implemented the versioned logical-workspace/local-checkout split.
- [ORB-10249] — implemented path-free task coordination and global task-relation/readiness lookup.
- [ORB-10255] — implemented ADR-0227's append-only SQLite host/alias core with
  compatible idempotent registration, permanent rename history, typed resolution,
  and non-deleting retirement.
- [ORB-10258] — implemented the enforcement half of ADR-0231 (Unit R1 of ORB-10246):
  origin-aware routine loading. Committed definitions fail closed without a non-empty host
  pin; `.orbit/routines/local/` definitions are implicit to the loading host and may not
  name another host; cross-origin name collisions fail deterministically.
- [ORB-10270] — completed ADR-0231 enforcement (Unit R2): registry/cache-aware validation
  runs before scheduler mutation, degraded cache evidence remains warning-only with exact
  local fallback, and reassignment preserves A while B baselines without backfill.
- [ORB-10267] — implemented Unit C3 of ORB-10246: operator host administration
  (`orbit host register/list/rename/retire`), hub-side workspace `link` and machine-local
  `role` (owner/replica) operations, the `orbit.host.list`/`orbit.workspace.list` canonical
  discovery tools, one path-free `RegistrySnapshotV1` projection read in a single store
  transaction, typed `workspace-required|global` tool scope metadata, the versioned atomic
  satellite registry cache (stable canonical payload plus local receipt; derived age/freshness
  does not create same-revision conflicts), and store schema v8's singleton
  `hub_registry_metadata` row carrying the hub `machine_id` and the monotonic
  `registry_revision` (advanced once per snapshot-visible mutation; no-ops do not advance it).
  Current-machine rename holds one machine-global lock across exact preflight, a staged,
  reparsed `host.toml` write, the durable registry rename, and post-error outcome
  classification. `machine_id` uses the canonical path-free `hm_` namespace, and hub-local
  CLI enumeration renders the same single-transaction sanitized snapshot as discovery.
  Remote registration and
  post-link/register cache refresh remain deferred to E3; poll-driven refresh to I/J.
- [ORB-10268] — implemented Unit E1's strict machine-global hub trust document and
  fixed checkoutless hub MCP endpoint, including repeated exact store-stamp checks,
  stable workspace-only payloads, canonical placement/capability filtering, and
  trusted one-call/one-audit provenance.
- [ORB-10302] — implemented Unit C4: moved the host/workspace/cache/service domain
  and its tests into `orbit-registry`, retired the replicated-registry scaffold,
  and preserved core/store/common ownership boundaries ([ADR-0235]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
