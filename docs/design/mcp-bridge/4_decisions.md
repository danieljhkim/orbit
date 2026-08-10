---
title: Orbit MCP Bridge — Decisions
owner: codex
last_updated: 2026-08-09
last_validated: 2026-08-02
status: Accepted
feature: mcp-bridge
doc_role: decisions
type: design
summary: ADR log for the coupled MCP Bridge and Host Registry v1 contract, its evolving implementation boundary, and the owned tunnel for checkoutless clients.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access]
related_artifacts: [ORB-00424, ORB-10245, ORB-10262, ORB-10267, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10276, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ORB-10690, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0348, ADR-0350, ADR-0351]
---

# Orbit MCP Bridge — Decisions

ADR log for `mcp-bridge`. Entries are append-only and ordered by global ID. The
Orbit ADR store owns allocation, status, and task links; this log records the
complete seven-decision v1 behavior contract shared with
[host-registry](../host-registry/4_decisions.md), plus the crate boundary that
keeps its implementation singular. ADR-0235 records the intermediate registry-only
extraction; accepted ADR-0240 records [ORB-10319]'s vertical Remote replacement.
The older entry remains intact as design history.

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Accepted · 2026-07 · [ORB-10245] accepted the coupled v1 contract; [ORB-10276] added the single projection-backed explicit-task-crew validation path: a non-empty `crew` on task add/update is validated against the resolved workspace owner's current stored execution profile (never hub-local crews, the registry cache, a stale replica, or a synchronous owner call), while an omitted or cleared crew still files without a profile and standalone/auto-task CRUD keep their local-runtime crew validation.

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

**Status:** Accepted · 2026-07 · [ORB-10245] froze tool routing and authorization; [ORB-10267] registered the first operator-only, hub-placement canonical discovery tools (`orbit.host.list`, `orbit.workspace.list`) in Remote's canonical composition and the versioned conformance fixture; [ORB-10262] implemented the local exact-checkout broker and capability enforcement; [ORB-10268] implemented the fixed checkoutless hub endpoint and exact scalar-capability surface; [ORB-10271] implemented active registered-caller validation, operator-only friction reads, and path-free remote artifacts/responses; [ORB-10319] colocated MCP-only discovery schemas and execution in Remote while retaining the dedicated human CLI commands; [ORB-10276] completed the discovery surface with the workspace-scoped, `{agent, operator}`, hub-placement `orbit.crew.list` (runner neither advertises nor executes it) beside the two operator-only global registry tools, reading the same profile projection through one service; [ORB-10332] removed the `orbit.host.list` MCP discovery tool as unused (the `orbit host list` CLI command and the `orbit.workspace.list` / `orbit.crew.list` MCP tools remain).

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

**Status:** Accepted · 2026-07 · [ORB-10245] fixed the one-writer knowledge rule; [ORB-10272] implemented the dormant Remote-v2 hub sequence, reconciliation, immutable-ledger, and atomic-audit substrate without activating the F3 public cutover; [ORB-10330] added the owner-side preallocated finalizers and the gated broker composition (one hub allocation, one exact-owner finalization, correlated by `mcp_call_id`; replica/foreign-spoke rejected before allocation) while public creation stays on the compatibility path until F3.

### Context

Knowledge needs global IDs without making a hub checkout or a stale replica a second
author.

### Decision

The hub allocates global IDs, the declared owner authors current knowledge, and Git
replicas are opt-in reads marked as replicas. The hub never proxies to a spoke owner.
Activation validates every registered workspace's complete hub-local legacy
inventory before mutation; missing sources or cross-workspace duplicate IDs fail
closed, and a late workspace remains ineligible until reconciled under the hub lock.

### Consequences

- A non-owner agent routes actionable work as a task to the owner.
- Sequence advancement, immutable correlation ledger, and canonical hub audit are
  one transaction; exact request-identity replay is idempotent.
- Standalone/worktree allocation remains unchanged until F3 activates and cuts over
  public knowledge creation.
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

**Replacement:** [ADR-0240] superseded this horizontal boundary when [ORB-10319]
landed. ADR-0235 remains the accepted historical decision.

## ADR-0240 — Consolidate remote coordination in one vertical feature crate

**Status:** Accepted · 2026-07 · [ORB-10319] implemented the boundary.

### Context

The registry-only extraction in ADR-0235 still made one MCP/remote feature cross
CLI, Core, Registry, Store, MCP, and Tools whenever persistence, schema composition,
and routing changed together. A separate broker crate would add another horizontal
layer without eliminating those cross-crate changes.

### Decision

Rename and widen `orbit-registry` into `orbit-remote`, a vertical feature crate that
owns host/workspace registry behavior, its SQLite statements and feature schema,
profiles and caches, MCP contract composition, the local broker, hub authority and
link, graph/learning integration, and registration. Keep `orbit-store` and
`orbit-mcp` as neutral kernels, shared DTOs in `orbit-common`, generic builtin
definitions in `orbit-tools`, and the transport-independent coordination executor
in `orbit-core`. Reuse the same config-resolved `orbit.db`; do not introduce a
Remote database or a separate broker crate.

### Consequences

- One Remote composition produces the production tool surface and hub digest, while
  the MCP kernel remains unaware of registry, graph, learning, or routing policy.
- Remote owns its registry SQL through `RemoteStore`; Store owns generic connection,
  transaction, and namespaced feature-migration infrastructure.
- Remote feature migration v2 owns the dormant hub-global knowledge sequence and
  reconciliation transaction [ORB-10272], while Store remains a neutral SQLite
  kernel.
- CLI retains command parsing, client setup/removal, and black-box binary tests;
  broker/hub/link/registration behavior evolves inside the feature crate.
- Cost: `orbit-remote` is intentionally broad and needs disciplined internal seams;
  genuinely cross-feature mechanisms must still be extracted into a neutral kernel.

## ADR-0350 — Own the SSH tunnel as remote-access infrastructure, with a provisional surface over it

**Status:** Proposed · 2026-08

### Context

The SSH-stdio hub link assumes a spoke: a machine with its own checkout, whose
graph, docs, and search must resolve against the branch its agent is working on.
An off-box orchestrator has no such checkout, so placement routing protects
nothing for it and only makes the canonical surface unreachable — which is what
forces the re-declared parity layer [ADR-0232] retires. Reachability, not tool
schemas, is the scarce resource for that client.

### Decision

Treat the SSH tunnel as owned, reusable infrastructure terminating at a
loopback-bound listener; calls resolve on the remote without placement routing,
for checkoutless clients only. What surface the tunnel carries is decided
separately by [ADR-0351].

### Consequences

- The canonical surface becomes reachable off-box without an external process
  re-declaring it, and cross-boundary drift is impossible because both ends are
  the same build.
- Separating transport from surface means the tunnel is worth building even if
  the surface question resolves unexpectedly; it is the part with no contingent
  value.
- Cost: Orbit now opens a listening port. The security property rests on a
  loopback bind guard rather than on the absence of a listener, so a
  misconfiguration binding a routable address is unauthenticated remote control.
- Cost: two cross-machine mechanisms coexist until one is retired.
- Cost: remote resolution is correct only for checkoutless clients; the
  refusal-when-a-checkout-exists guard is load-bearing, and its absence presents
  as wrong answers rather than as an error.

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0350"}'`.

## ADR-0351 — Serve the remote surface as enumerate, invoke-by-name, and claim-gated command execution

**Status:** Proposed · 2026-08

### Context

Reachability is scarce: an orchestrator that cannot execute on the machine routes
trivial reads through full worker runs. Establishing the tunnel presupposes SSH,
and anyone with SSH can already run anything there, so withholding command
execution from such a caller protects nothing. But unrestricted command execution
in the default surface would make capability filtering, the governed-operation
check, and the workspace claim advisory for whoever holds it. Note that Orbit's
advertised definitions are derived from the tool registry, not hand-maintained —
the duplication this feature removes was an external process re-declaring them.

### Decision

Serve three operations over the tunnel: enumerate the registry entries visible to
the caller with their schemas; invoke a tool by name through the governed
chokepoint; and run a command as argv plus working directory, requiring operator
capability and the workspace claim and withheld from managed runs. A client
without the claim receives enumerate and invoke, never command — the boundary is
which operations exist for that caller, not an allowlist over argv, which leaks.
The existing advertised per-tool surface is retained pending tool-call metrics.

### Consequences

- The orchestrator stops dispatching a full worker run to answer questions a
  single command answers, and every registry operation stays reachable without a
  per-operation adapter.
- Splitting invoke from command preserves per-tool audit attribution that a
  command-only surface would have discarded.
- Policy and placement metadata move from filtering an advertised list to
  authorizing an invocation — enforcement rather than advertisement.
- Cost: enumerate and invoke rebuild the protocol's own list/call verbs inside a
  tool, justified only by collapsing per-tool policy into one authorization point.
- Cost: for a claim-holding client, capability filtering above command is
  advisory. Gating bounds who that applies to; it does not make it untrue.
- Cost: audit granularity degrades for command calls specifically.
- Cost: the surface stays doubled through the measurement period.

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0351"}'`.

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10267] — registered the `orbit.host.list` (later removed in [ORB-10332]) and
  `orbit.workspace.list` operator discovery
  tools (hub placement, operator capability, typed global/workspace-unscoped scope) in the
  Remote-owned discovery registry and the versioned conformance fixture, with every pre-existing tool
  defaulting to typed `workspace-required`. Each discovery tool is backed by one sanitized,
  path-free registry snapshot. C3 proves the real broker/store action path with no session
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
  recognition-only in hub composition, checkoutless writes use stable workspace IDs, and every denial or
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
- [ORB-10319] — implements proposed ADR-0240 by renaming and widening the registry
  crate into vertical `orbit-remote`, adopting registry persistence in place and
  moving MCP composition/broker/hub/link/registration out of CLI and the MCP kernel.
- [ORB-10272] — implements ADR-0229's dormant Remote-v2 hub allocation substrate:
  complete validated legacy reconciliation, forward-only activation, independent
  sequences, immutable correlation ledger plus atomic audit, no owner proxy, and
  explicit late-workspace ineligibility. Its private path-free allocation protocol
  advances the connector contract to revision 3. Public issuance remains an F3
  cutover and standalone creation remains unchanged.
- [ORB-10276] — completed host/workspace/crew discovery and the first
  projection-backed validation path (Unit H1). Registered the workspace-scoped,
  `{agent, operator}`, hub-placement `orbit.crew.list` beside the two operator-only
  global registry tools and in the conformance fixture, and added one reusable
  `orbit-remote` execution-profile projection service (injected clock, single shared
  freshness TTL) that both crew discovery and explicit task-crew validation read.
  A non-empty task crew on add/update now requires the resolved workspace owner's
  current stored profile and validates against its effective crews; missing/stale
  profiles and unknown crews fail with actionable workspace/owner/state/age errors
  and mutate nothing. Standalone and owner-local auto-task CRUD keep their local
  crew validation. Task `host`/claims (H2), workflow ship/placement (H3), and run
  lineage/leasing (I1) remain out of scope.
- [ORB-10332] — removed the `orbit.host.list` MCP discovery tool as unused; the
  `orbit host list` CLI command and the `orbit.workspace.list` / `orbit.crew.list`
  MCP discovery tools remain.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
