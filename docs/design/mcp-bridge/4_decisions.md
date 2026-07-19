---
title: Orbit MCP Bridge — Decisions
owner: codex
last_updated: 2026-07-18
status: Accepted
feature: mcp-bridge
doc_role: decisions
type: design
summary: Accepted ADR log for the coupled MCP Bridge and Host Registry v1 contract.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-mcp/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-registry/**", "crates/orbit-core/src/command/tool.rs"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access]
related_artifacts: [ORB-00424, ORB-10245, ORB-10262, ORB-10267, ORB-10268, ORB-10269, ORB-10271, ORB-10302, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235]
---

# Orbit MCP Bridge — Decisions

ADR log for `mcp-bridge`. Entries are append-only and ordered by global ID. The
Orbit ADR store owns allocation, status, and task links; this log records the
complete seven-decision v1 behavior contract shared with
[host-registry](../host-registry/4_decisions.md), plus the crate boundary that
keeps its registry implementation singular.

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Accepted · 2026-07 · [ORB-10245] accepted the coupled v1 contract.

### Context

Cross-machine work needs a coordination authority, a knowledge author, and an
execution destination; making them one authority would scatter task state or make
ownership implicit.

### Decision

Use exactly one coordination hub for every workspace, declare one workspace owner,
and select execution placement per run with the owner as the default.

### Consequences

- Coordination writes remain hub-routed while knowledge authorship remains owner-bound.
- Cost: hub downtime stalls coordination for every workspace, and disconnected machines cannot write coordination records.

## ADR-0227 — Stable machine identity, registry, and out-of-band hub pin

**Status:** Accepted · 2026-07 · [ORB-10245] froze the host identity boundary;
[ORB-10268] implemented the strict machine-global trust document and exact hub/store pin;
[ORB-10271] implemented private spoke self-registration from validated local identity,
honest committed stages, and definitive-success cache refresh.

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

**Status:** Accepted · 2026-07 · [ORB-10245] froze tool routing and authorization; [ORB-10267] registered the first operator-only, hub-placement canonical discovery tools (`orbit.host.list`, `orbit.workspace.list`) in the canonical registry and the versioned conformance fixture; [ORB-10262] implemented the local exact-checkout broker and capability enforcement; [ORB-10268] implemented the fixed checkoutless hub endpoint and exact scalar-capability surface; [ORB-10271] implemented active registered-caller validation, operator-only friction reads, and path-free remote artifacts/responses.

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

**Status:** Accepted · 2026-07 · [ORB-10302] established the coupled implementation boundary.

### Context

The bridge needs one registry-domain API below `orbit-core`, not a second set of
MCP DTOs and not the retired opaque-byte replication substrate that previously
occupied `orbit-registry`.

### Decision

Place identity, local workspace roles/catalog, cache semantics, and the
store-backed service in `orbit-registry`; keep MCP serialization/dispatch in the
adapter, runtime profile/ship construction in `orbit-core`, persistence in
`orbit-store`, and shared DTOs in `orbit-common`.

### Consequences

- Future broker and transport work consumes one domain surface without moving SQL or routing policy into it.
- The retired replicated-writer API cannot reappear as a second authority beside the singular hub.
- Cost: `orbit-registry` gains a store dependency and is no longer a consumer-agnostic leaf.

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10267] — registered the `orbit.host.list` and `orbit.workspace.list` operator discovery
  tools (hub placement, operator capability, typed global/workspace-unscoped scope) in the
  canonical builtin registry and the versioned conformance fixture, with every pre-existing tool
  defaulting to typed `workspace-required`. Each discovery tool is backed by one sanitized,
  path-free registry snapshot. C3 proves the real runtime/store action path with no session
  workspace or checkout binding for the enumerated workspace.
- [ORB-10262] — replaced startup cwd discovery and `EmptyMcpHost` with the canonical
  schema-listing broker, exact-checkout resolution/cache identity, placement preflight, and
  effective-session `tools/list` omission plus hidden-name `tools/call` denial. Hub coordination
  calls now execute by stable workspace ID without `OrbitRuntime` or a fabricated checkout;
  review persistence is independent of local scoreboard/model configuration, and canonical friction state uses marker-committed
  migration under the global workspace partition.
- [ORB-10268] — added strict `~/.orbit/mcp.toml` trust parsing and the non-recursive
  `orbit mcp serve --hub` endpoint. Hub startup/list/call verify the exact
  `host.toml`/store stamp, canonical hub/capability filtering is singular, graph is
  not re-merged, checkoutless writes use stable workspace IDs, and every denial or
  outcome keeps one trusted D2 audit identity.
- [ORB-10269] — added the fixed SSH argv connector, per-capability bounded link
  pool, revision plus canonical hub-schema negotiation, trusted remote call
  metadata, and the pre-handoff `hub_unavailable` / post-handoff
  `outcome_unknown` split. Mutations are never replayed automatically.
- [ORB-10271] — added the connector-private registration protocol, contract
  revision 2, staged registry/projection/snapshot results, active caller checks,
  definitive-success cache refresh, path-free artifact/friction handling, and the
  two-root RMCP canary proving hub-only writes and one trusted audit per call.
- [ORB-10302] — moved the coupled registry domain into `orbit-registry` and retained
  MCP ownership of serialization/dispatch only ([ADR-0235]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
