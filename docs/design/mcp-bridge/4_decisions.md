---
title: Orbit MCP Bridge — Decisions
owner: claude
last_updated: 2026-08-10
last_validated: 2026-08-02
status: Draft
feature: mcp-bridge
doc_role: decisions
type: design
summary: ADR log for mcp-bridge, including the retired singular-hub contract and its v1 ownership-model replacement, the evolving implementation boundary, and the owned tunnel for checkoutless clients.
tags: [mcp, remote-access, host-registry, bridge]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access]
related_artifacts: [ORB-00424, ORB-10245, ORB-10708, ORB-10710, ORB-10262, ORB-10267, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10276, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ORB-10690, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0348, ADR-0350, ADR-0351, ADR-0352, ADR-0354, ADR-0355, ADR-0356, ADR-0357, ADR-0358]
---

# Orbit MCP Bridge — Decisions

> **Status: Draft — structural rewrite in flight.** The singular-hub contract
> ([ADR-0226], [ADR-0229], [ADR-0230]) is superseded by [ADR-0355]–[ADR-0358],
> which are recorded in
> [../host-registry/4_decisions.md](../host-registry/4_decisions.md) because that
> is the feature they primarily govern. Entries below that describe execution
> placement, run leases, and host registration are **deferred to v2**.

ADR log for `mcp-bridge`, in ascending number order. Entries are append-only,
numbered per-repo, and committed here: this file is the record, and there is no
external ADR store behind it — see
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict) for why that surface was
retired. A superseded decision keeps its heading and body so the reason the earlier
architecture existed is not rewritten after the fact.

ADR-0226 through ADR-0232 were the consolidated v1 behaviour contract shared with
[host-registry](../host-registry/4_decisions.md), built around a single coordination
hub. **Four of them are superseded** by ADR-0355–ADR-0358, which replace the
singular hub with per-machine coordination. ADR-0235 records the intermediate
registry-only extraction and ADR-0240 the vertical Remote boundary that replaced it;
both are unaffected, as are ADR-0232, ADR-0350, ADR-0351, and ADR-0354.

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Superseded by [ADR-0355] · 2026-08 · originally accepted 2026-07 — [ORB-10245] accepted the coupled v1 contract; [ORB-10276] added the single projection-backed explicit-task-crew validation path: a non-empty `crew` on task add/update is validated against the resolved workspace owner's current stored execution profile (never hub-local crews, the registry cache, a stale replica, or a synchronous owner call), while an omitted or cleared crew still files without a profile and standalone/auto-task CRUD keep their local-runtime crew validation.

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

**Superseded because the cost line understated the problem.** Routing every
workspace through one hub was not only an availability dependency — it was
mandatory. A machine-level `mode` meant a laptop could not keep a purely local
project local while still getting unique task IDs, because uniqueness was a
property of the hub's single allocator. [ADR-0355] and [ADR-0356] separate those
two things. For the bridge specifically, the `hub` placement class collapses into
`owner` and the "one cross-machine destination" invariant becomes "at most one
destination per call, and it is the workspace's owner."

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

**Partially retained.** The `machine_id` contract, the names-resolve-at-binding
rule, and the out-of-band pinning of a route target all survive verbatim; v1 only
generalizes "the one hub" to "each route," so `mcp.toml` names zero or more owner
machines instead of one hub. The *registry* half — the hub-side inventory, name
reservation across the fleet, and tombstone aliases — is deferred with [ADR-0358],
which means v1 has no cross-machine name reservation at all and nothing validates
that a claimed caller `machine_id` exists. [ORB-10271]'s private spoke
self-registration is withdrawn with it.

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

**Amended by [ADR-0355] and [ADR-0358].** The broker survives, and so does the rule
that every exposed tool carries exactly one placement and an independently filtered
capability set. What changes is the enumeration: the `hub` class is withdrawn and
its tools become `owner`, meaning "the coordinating machine for this tool's
workspace," which for an owned workspace is in-process. The remaining classes are
`owner`, `local-derived`, and `composite`. The `runner` capability set is withdrawn
with execution placement, and the `orbit.workspace.list` / `orbit.crew.list`
discovery tools now read local registry and config data rather than a fleet
snapshot. [ADR-0350] separately narrows the placement broker to clients that hold
local-derived state.

## ADR-0229 — Owner-authored knowledge with hub-global IDs and explicit replicas

**Status:** Superseded by [ADR-0357] · 2026-08 · originally accepted 2026-07 — [ORB-10245] fixed the one-writer knowledge rule; [ORB-10272] implemented the dormant Remote-v2 hub sequence, reconciliation, immutable-ledger, and atomic-audit substrate without activating the F3 public cutover; [ORB-10330] added the owner-side preallocated finalizers and the gated broker composition (one hub allocation, one exact-owner finalization, correlated by `mcp_call_id`; replica/foreign-spoke rejected before allocation) while public creation stays on the compatibility path until F3.

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

**Superseded because the global ID was never needed.** All of the machinery above —
reconciliation, the allocation ledger, the dormant/active marker, the
allocate-then-finalize composition — exists to make one number unique across
machines. [ADR-0357] observes that knowledge records are already addressed within a
workspace and keys them `(workspace_id, artifact_key)`, at which point the entire
protocol evaporates: no reservation, no expiry, no orphaned ID, no finalize/pull
race, and no composite placement for `learning.add`. F3 never ran, so no ID was ever
issued from the hub sequence and nothing needs renumbering. The
one-writer-per-workspace rule this ADR established survives intact. The `orbit.adr.*`
tool family goes with the ADR store itself
([CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict)).

## ADR-0230 — Pull-based leases with immutable placement and explicit recovery

**Status:** Superseded by [ADR-0358] · 2026-08 · originally accepted 2026-07 — [ORB-10245] fixed runner delivery semantics.

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

**Superseded by withdrawal, not by a better protocol.** The lease design is sound
and [ADR-0358] does not replace it with anything — v1 simply has no case for running
a task anywhere but the machine that owns its workspace. For the bridge this removes
the `orbit.run.lease/report/presence` tool family, the `runner` capability, and the
`leased_run` session field. The mailbox posture and the immutable requested/actual
snapshot are the parts to reread if placement returns
([../host-registry/3_vision.md](../host-registry/3_vision.md)).

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

**Amended by [ADR-0358], not superseded.** The ownership rule and host-local cursors
are unchanged. What weakens is *validation*: with no fleet registry, a pin resolves
against local `host.toml` and the owner names recorded in this machine's own
workspace registry only, so an unrecognized name cannot be distinguished from a typo
and there is no `last_seen` to notice a quiet owner. The own-host case — the one that
decides whether anything fires — stays decidable offline, which was always the
load-bearing half.

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
- The retired replicated-writer API cannot reappear as a second authority beside the singular hub. (Under [ADR-0355] the invariant it protected is restated as one coordination writer *per workspace*, which is what the replicated-writer model actually violated; the crate boundary is unchanged either way.)
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
profiles and caches, MCP contract composition, the local broker, the owner route,
and graph/learning integration. Keep `orbit-store` and
`orbit-mcp` as neutral kernels, shared DTOs in `orbit-common`, generic builtin
definitions in `orbit-tools`, and the transport-independent coordination executor
in `orbit-core`. Reuse the same config-resolved `orbit.db`; do not introduce a
Remote database or a separate broker crate.

### Consequences

- One Remote composition produces the production tool surface and hub digest, while
  the MCP kernel remains unaware of registry, graph, learning, or routing policy.
- Remote owns its registry SQL through `RemoteStore`; Store owns generic connection,
  transaction, and namespaced feature-migration infrastructure.
- Remote feature migration v2 owned the dormant hub-global knowledge sequence and
  reconciliation transaction [ORB-10272], while Store remained a neutral SQLite
  kernel. **Amended:** [ADR-0357] removes global knowledge IDs, so that migration is
  reverted rather than carried forward — the crate boundary this ADR establishes is
  unaffected, only one of its tenants is.
- CLI retains command parsing, client setup/removal, and black-box binary tests;
  broker and owner-route behavior evolves inside the feature crate.
- Cost: `orbit-remote` is intentionally broad and needs disciplined internal seams;
  genuinely cross-feature mechanisms must still be extracted into a neutral kernel.

## ADR-0350 — Own the SSH tunnel as remote-access infrastructure, with a provisional surface over it

**Status:** Accepted · 2026-08 · [ORB-10690] implemented the loopback listener with one server instance per connection; [ORB-10710] added the CLI surface, the client-side `--mode remote`, and the checkout guard.

### Context

Orbit's canonical MCP surface reaches a remote machine one way today: a spoke
broker spawns `ssh <alias> orbit mcp serve --hub` and relays frames over that
process's stdio. The stated posture is that Orbit opens no listening port and
invents no credential of its own.

That path assumes the client is a spoke — a machine with its own checkout, whose
graph, docs, and search must resolve against the branch its agent is working on.
Placement classes (`hub`, `owner`, `local-derived`, `composite`) exist to preserve
exactly that.

A second client class does not fit the assumption. An off-box orchestrator has no
meaningful local checkout: its clone, if any, is a read mirror, and every workspace
it acts on lives on the remote. There is no local-derived state to protect, so
placement routing guards nothing for it and only makes the canonical surface
unreachable. The observed consequence is the parity layer this feature already
decided to retire — an external process that re-declares Orbit's tools in another
language against the dashboard HTTP API, discards Orbit's capability model, and
drifts on every schema change. That duplication is across a process boundary;
Orbit's own advertised definitions are derived from its tool registry, not
hand-copied, and are not the problem being solved here.

Reachability is the scarce thing. An orchestrator that cannot reach the machine
currently launders trivial reads through full worker runs.

### Decision

Treat the SSH tunnel as owned infrastructure, and decide separately what it
carries.

- Orbit establishes or reuses an SSH tunnel to a **loopback-bound** listener on the
  remote machine. The listener refuses any non-loopback bind, exactly as the
  dashboard does. SSH owns authentication, encryption, and host verification; Orbit
  adds no credential, ACL, or session of its own. This is the same delegation the
  hub link already makes, applied to a tunnel rather than a spawned process.
- The tunnel is a reusable primitive, not an implementation detail of one consumer.
  Anything that needs to reach the remote machine rides it rather than opening a
  second mechanism.
- Calls carried over the tunnel resolve **on the remote**, without placement
  routing. That is correct precisely because the client holds no local-derived
  state, and it is why the mode must refuse to start where a local checkout exists
  rather than silently answering from another machine's branch.
- Placement routing is unchanged for spokes. This narrows the placement broker's
  scope to clients that hold local-derived state; it does not supersede that
  decision.
- **What surface the tunnel carries is decided separately, by [ADR-0351].** This
  record commits only to the transport and its trust posture. Forwarding the
  existing advertised per-tool surface is one thing the tunnel may carry, not the
  reason it exists.

### Consequences

- The canonical surface becomes reachable off-box without an external process
  re-declaring it, so schema drift across the process boundary stops being
  possible: both ends are the same build.
- Capability filtering and audit apply to remote callers through the paths that
  already implement them, rather than needing equivalents rebuilt on a second
  surface.
- Separating the transport decision from the surface decision means the tunnel is
  worth building even if the surface question resolves differently than expected.
  It is the part of this work with no contingent value.
- **Cost:** Orbit now opens a listening port, contradicting a previously absolute
  posture. Loopback binding plus a tunnel preserves the security property, but that
  guarantee now rests on a bind guard rather than on the absence of a listener —
  and a misconfiguration binding a routable address turns the surface into
  unauthenticated remote control of the machine.
- **Cost:** a second cross-machine mechanism exists beside the SSH-stdio hub link.
  Until one is retired, two paths reach a remote Orbit, which is the duplication
  this feature was created to remove. The tunnelled listener is deliberately not a
  hub link and must not acquire hub-link responsibilities: no placement routing, no
  workspace-ownership resolution, no spoke registration.
- **Cost:** remote resolution is correct only for the client class this is defined
  for. The refusal-when-a-checkout-exists guard is load-bearing; without it the mode
  returns another machine's branch state as though it were local, which presents as
  wrong answers rather than as an error.
- The star topology's "one cross-machine destination" invariant now describes spokes
  specifically. A checkoutless client is not a spoke and does not participate in hub
  or owner routing.

**Amended by [ADR-0355] and [ADR-0358], not superseded.** The transport decision and
its trust posture stand exactly as written. What changed underneath is the framing:
with the singular hub gone, the owned tunnel is the *primary* cross-machine route
rather than an exception carved out of a star topology. Three consequences follow.
The final bullet above is void — there is no star topology and no spoke class, so
"one cross-machine destination" is now simply "at most one destination per call,
and it is the workspace's owner." The second cost resolves: the SSH-stdio hub link
is retired, so only one cross-machine mechanism remains. And "not a hub link" is
better read as "a transport, not an authority" — ownership preflight is a property
of the machine that serves a call, never of the pipe that carried it.

## ADR-0351 — Expose remote command execution as a claim-gated tool, retaining the advertised surface

**Status:** Proposed · 2026-08 · a committed heading in this file, not a record in a store; `Proposed` holds only while the implementing task is in flight and on its branch ([CONVENTIONS.md §4c](../CONVENTIONS.md#4c-format-and-numbering)).

### Context

With the tunnel owned as infrastructure, an off-box orchestrator can reach the
remote machine. The question is what it should be able to do there.

A correction first, because an earlier draft of this record overstated the problem.
Orbit's MCP tool definitions are **not** hand-maintained duplicates of the CLI. They
are derived from the tool registry — a tool is written once in Rust with its schema,
registered with a policy, and the advertised surface is computed from those entries.
The duplication this feature was created to remove was an external process
re-declaring those schemas in another language across a process boundary, and the
owned tunnel already eliminates it. What the advertised surface actually costs is
per-tool policy and placement metadata, the conformance test pinning the definition
count, the contract digest, and the context those definitions occupy in every client
request. Real, but modest.

What is genuinely scarce is reachability. An orchestrator that cannot execute on the
machine routes trivial reads through full worker runs — disproportionate to the
work, and slow enough to distort how often such checks happen at all.

There is also a boundary to respect. A client that can run arbitrary commands can
invoke the CLI, and the CLI reaches every operation the capability model governs,
including workflow dispatch. Unrestricted command execution in the default surface
would make capability filtering, the governed-operation check, and the workspace
claim advisory for whoever holds it. Against that: establishing the tunnel already
presupposes SSH to the machine, and anyone with SSH can already run anything there.

### Decision

Add command execution, and change nothing else about the surface.

- **Command** takes an argv array and an explicit working directory. Never a shell
  string, so quoting and operator-precedence bugs are structurally impossible rather
  than merely discouraged.
- It requires **operator capability and the workspace claim**, and is withheld from
  managed runs, which could otherwise bypass the self-dispatch guard through the
  CLI.
- A client without the claim does not receive command at all. The restriction is
  **not** an allowlist over argv: a filtered command surface leaks through
  `bash -c`, `env`, `xargs`, `make`, interpreter `-c` flags, and version-control
  hooks, so the boundary is whether the operation exists for that caller, not which
  binaries it may name.
- **The advertised per-tool surface is unchanged.** Clients keep native tool
  selection, call-time argument validation, and per-tool audit attribution. Routine
  work continues to be attributed by tool name; only genuinely arbitrary execution
  degrades to an argv.

Replacing the advertised surface with generic enumerate and invoke-by-name
operations is deliberately **not** decided here. It remains open, and the cost of
keeping it open is one additional path to the same operations.

### Consequences

- The orchestrator stops dispatching a full worker run to answer questions a single
  command answers, and new CLI capability is reachable the moment it ships rather
  than after a schema is mirrored.
- Per-tool audit attribution is preserved for everything except command itself,
  which is the narrowest possible degradation of provenance.
- Nothing is foreclosed. The advertised definitions are generated from the registry,
  so removing them later is a revert rather than a rebuild — that, not a
  measurement, is what makes this reversible.
- **Cost:** for a claim-holding client, capability filtering above command is
  advisory — it can invoke the CLI and reach any governed operation. Requiring both
  operator capability and the claim, and withholding it from managed runs, bounds
  who that applies to; it does not make it untrue.
- **Cost:** audit granularity degrades for command calls specifically. An argv is
  not a tool name, and workspace correlation becomes conventional rather than
  structural.
- **Cost:** two paths now reach the same operations — the advertised tool and the
  CLI through command. That duplication is accepted deliberately rather than by
  oversight.
- **Cost:** deciding later whether the advertised surface earns its place requires
  evidence that no current endpoint produces. `/metrics/tools` is an ungrouped
  invocation count with no caller dimension; the usable cut is over audit events,
  excluding rows carrying a job-run or activity id so that engine and worker traffic
  does not swamp the orchestrator's. Until someone builds that cut, retaining the
  surface is deferral, not measurement, and this record should not pretend
  otherwise.

**Unaffected by the v1 ownership model.** The workspace claim this decision gates on
is [ADR-0352], which is retained and orthogonal to ownership: ownership binds a
workspace to a machine, the claim binds dispatch authority to an operator session.

## ADR-0354 — Own the SSH local-forward tunnel once, at the leaf, shared by every loopback listener

**Status:** Accepted · 2026-08 · [ORB-10710] moved the mechanism to `orbit-common` and made both surfaces consume it.

### Context

[ADR-0350] commits to the SSH tunnel as *reusable infrastructure*: "anything that
needs to reach the remote machine rides it rather than opening a second mechanism."
At the point [ORB-10710] added the second consumer, that reuse was not structurally
possible.

The only attach-or-spawn tunnel in the tree lived in `orbit-dashboard::connect`
([ORB-10708]): pick a local port, open a bare `ssh -N` forward, probe through it,
attach if something already answers, otherwise run a second `ssh` that both forwards
and starts the remote process, and tear down on drop only what this invocation
started. Roughly 150 lines, and every line of it is exactly what `orbit mcp serve
--mode remote` needs.

`orbit-dashboard` already depends on `orbit-remote`. The proxy lives in
`orbit-remote`, so it cannot call into the dashboard: that edge runs the wrong way,
and reversing it would invert the layering for a process-spawning helper.

### Decision

Move the mechanism to `orbit-common::utility::ssh_tunnel` and make both surfaces
consume it. The module owns the `SshTunnel` RAII child, teardown, port selection,
forward-argument construction, `shell_quote`, `ssh` exit classification, readiness
polling, and the attach-first `establish` sequence.

Each consumer keeps only what is genuinely its own: the dashboard keeps its
`/healthz` probe, its remote `orbit web serve` command line, and its browser and
shutdown behavior; the proxy keeps its TCP readiness probe, its `orbit mcp serve
--listen` command line, and its checkout guard. A `TunnelSpec` carries the caller's
remote command and its two timeouts, so the shared module never composes what runs
on the far side.

The module is deliberately synchronous and `std`-only. A tunnel is a process
lifetime, not a future; consumers own their own runtime, or have none.

The leaf is the placement, not `orbit-exec` (whose process primitives are about
sandboxed command execution under an `FsProfile`) and not a new crate. Both
consumers already depend on `orbit-common`, so this adds **no new dependency edge**.

### Consequences

- The "one mechanism" property in [ADR-0350] is now structural rather than
  aspirational: a third loopback listener reaching for a tunnel finds one
  implementation, and a fix to teardown or attach semantics lands for every consumer
  at once.
- Attach-first behavior — the part that makes a long-lived remote listener usable at
  all — is inherited by the proxy rather than reimplemented, so the two surfaces
  cannot drift on whether disconnecting kills a pre-existing remote process.
- Generic behavior is tested once, in `orbit-common`; each consumer's tests shrink
  to the part it actually owns.
- **Cost:** `orbit-common` is a `stable`-tier leaf and now spawns processes. That is
  a genuine widening of what "shared utility" means there, justified only because
  the alternative placements are worse: duplicating ~150 lines across two crates
  exceeds the duplication threshold, and an `orbit-remote -> orbit-dashboard` edge
  inverts the layering.
- **Cost:** the dashboard's timeout and `ssh`-exit messages are now composed from a
  shared template plus a caller-supplied description, so their exact wording changed
  slightly. Operator-facing strings are no longer owned end-to-end by the command
  that emits them.
- **Rejected:** duplicating the helpers into `orbit-remote` and filing a follow-up
  consolidation task. It is the cheaper edit and the standard escape hatch for
  cross-crate duplication, but it contradicts [ADR-0350]'s explicit "rather than
  opening a second mechanism" the moment the second consumer exists — the exact
  point at which consolidating is still cheap.
- **Rejected:** a dedicated `orbit-tunnel` crate. Correct if a third consumer with
  different transport needs appears; today it buys isolation nothing currently needs
  at the cost of a crate in the graph.

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10267] — registered the `orbit.host.list` (later removed in [ORB-10332]) and
  `orbit.workspace.list` operator discovery
  tools (hub placement, operator capability, typed global/workspace-unscoped scope) in the
  Remote-owned discovery registry and the versioned conformance fixture, with every pre-existing tool
  defaulting to typed `workspace-required`. Each discovery tool is backed by one sanitized,
  path-free registry snapshot. C3 proves the real broker/store action path with no session
  workspace or checkout binding for the enumerated workspace. Superseded by the v1
  ownership model: hub placement becomes `local-derived` over the machine-local
  registry, and the fleet-inventory tool has nothing to enumerate ([ADR-0358]).
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
  outcome keeps one trusted D2 audit identity. Superseded in part by the v1
  ownership model: the trust document survives as the client's per-route policy and
  the endpoint as the owner endpoint, but the `--hub` flag and the single fixed
  target do not.
- [ORB-10269] — added the fixed SSH argv connector, per-capability bounded link
  pool, revision plus canonical hub-schema negotiation, trusted remote call
  metadata, and the pre-handoff `hub_unavailable` / post-handoff
  `outcome_unknown` split. Mutations are never replayed automatically. The
  transport survives the v1 ownership model; the single hub target becomes a
  per-owner route and `hub_unavailable` becomes `owner_unavailable`.
- [ORB-10271] — added the connector-private registration protocol, contract
  revision 2, staged registry/projection/snapshot results, active caller checks,
  definitive-success cache refresh, path-free artifact/friction handling, and the
  two-root RMCP canary proving hub-only writes and one trusted audit per call.
  Superseded by the v1 ownership model: registration, the active-caller guard, and
  the spoke cache are withdrawn with the fleet registry ([ADR-0358]); the path-free
  frames and one-audit-per-call discipline survive.
- [ORB-10302] — moved the coupled registry domain into `orbit-registry` and retained
  MCP ownership of serialization/dispatch only ([ADR-0235]).
- [ORB-10319] — implements proposed ADR-0240 by renaming and widening the registry
  crate into vertical `orbit-remote`, adopting registry persistence in place and
  moving MCP composition/broker/hub/link/registration out of CLI and the MCP kernel.
- [ORB-10272] — implemented ADR-0229's dormant Remote-v2 hub allocation substrate:
  complete validated legacy reconciliation, forward-only activation, independent
  sequences, immutable correlation ledger plus atomic audit, no owner proxy, and
  explicit late-workspace ineligibility. Its private path-free allocation protocol
  advanced the connector contract to revision 3. Superseded by the v1 ownership
  model and **removed rather than parked** ([ADR-0357]): it encodes a model that no
  longer exists. Public issuance never activated, so no ID was ever allocated from
  it and nothing needs renumbering.
- [ORB-10330] — implemented the F2 owner preallocated finalizers and the gated
  hub-allocate/owner-finalize broker composition. Superseded with the allocator:
  with no allocation step there is no preallocated ID to finalize.
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
  lineage/leasing (I1) remain out of scope. Superseded in part by the v1 ownership
  model: the profile *publication* path is withdrawn with registration, and crew
  validation reads the owner machine's local config instead ([ADR-0358]). The two
  digests and `build_execution_profile_v1` survive as transport-independent
  construction.
- [ORB-10332] — removed the `orbit.host.list` MCP discovery tool as unused; the
  `orbit.workspace.list` / `orbit.crew.list` MCP discovery tools remain. The
  `orbit host list` CLI command it deferred to is itself withdrawn with the fleet
  inventory ([ADR-0358]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
