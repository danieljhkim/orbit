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
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge]
related_artifacts: [ORB-00424, ORB-10245, ORB-10248, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232]
---

# Host Registry — Decisions

ADR log for `host-registry`. Entries are append-only and ordered by global ID.
The Orbit ADR store owns their allocation, status, and task link; this document is
the long-form feature log. The seven entries below are the complete consolidated
v1 decision set shared with [mcp-bridge](../mcp-bridge/4_decisions.md).

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Accepted · 2026-07 · [ORB-10245] accepted the coupled v1 contract; [ORB-10248] implemented the workspace boundary.

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
- Cost: hub downtime stalls coordination for every workspace, and disconnected machines cannot write coordination records.

## ADR-0227 — Stable machine identity, registry, and out-of-band hub pin

**Status:** Accepted · 2026-07 · [ORB-10245] froze the host identity boundary.

### Context

Hostname-derived strings and per-workspace transport targets can silently redirect a
machine or elevate repository configuration.

### Decision

Give each machine an immutable generated `machine_id`, keep the registry at the hub,
and pin the one hub `machine_id` out of band in machine-local `mcp.toml`.

### Consequences

- Names resolve at binding time and persisted records retain stable identity.
- Cost: bootstrap transfers hub identity out of band, and registry/trust drift needs explicit diagnosis.

## ADR-0228 — Local placement broker with capability-set filtering

**Status:** Accepted · 2026-07 · [ORB-10245] froze tool routing and authorization.

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

**Status:** Accepted · 2026-07 · [ORB-10245] fixed routine execution ownership.

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

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10248] — implemented the versioned logical-workspace/local-checkout split.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
