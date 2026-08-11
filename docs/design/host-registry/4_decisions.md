---
title: Host Registry — Decisions
owner: claude
last_updated: 2026-08-11
last_validated: 2026-08-11
status: Draft
feature: host-registry
doc_role: decisions
type: design
summary: Decision record for host identity, per-machine coordination, prefix-partitioned task IDs, workspace-scoped knowledge keys, and the deferral of fleet registration and execution placement.
tags: [host-registry, mcp-bridge, multi-host, ownership]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge]
related_artifacts: [ORB-00424, ORB-10245, ORB-10248, ORB-10249, ORB-10255, ORB-10257, ORB-10267, ORB-10258, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10276, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ORB-10709, ORB-10723, ORB-10728, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0352, ADR-0355, ADR-0356, ADR-0357, ADR-0358]
---

# Host Registry — Decisions

Decision record for `host-registry`, in ascending number order. Entries are
append-only: a superseded decision keeps its heading and body so the reason the
earlier architecture existed is not rewritten after the fact. This file is the
authoritative body — see [CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict) for
why there is no longer a store behind it.

ADR-0226 through ADR-0232 were the consolidated v1 behaviour contract shared with
[mcp-bridge](../mcp-bridge/4_decisions.md), built around a single coordination hub.
**Four of them are superseded** by ADR-0355–ADR-0358, which replace the singular
hub with per-machine coordination. ADR-0235 records the first registry-only crate
extraction and ADR-0240 the vertical Remote boundary that replaced it; both are
unaffected, as is ADR-0352.

## ADR-0226 — Singular coordination hub, workspace owner, and per-run placement

**Status:** Superseded by [ADR-0355] · 2026-08 · originally accepted 2026-07 — [ORB-10245] accepted the coupled v1 contract; [ORB-10248] implemented the workspace boundary; [ORB-10249] applied it to task coordination.

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

**Superseded because the cost line understated the problem.** Routing every
workspace through one hub was not only an availability dependency — it was
mandatory. A machine-level `mode` meant a laptop could not keep a purely local
project local while still getting unique task IDs, because uniqueness was a
property of the hub's single allocator. [ADR-0355] and [ADR-0356] separate those
two things. The workspace/checkout split this ADR introduced ([ORB-10248],
[ORB-10249]) survives unchanged and turned out to be the right substrate.

## ADR-0227 — Stable machine identity, registry, and out-of-band hub pin

**Status:** Accepted · 2026-07 · [ORB-10245] froze the host identity boundary; [ORB-10255] implemented the durable registry core; [ORB-10267] added operator administration (register/list/rename/retire), current-machine `host.toml` rename coordination, and the hub-global `registry_revision`; [ORB-10268] enforced the machine-global trust and exact hub/store pin at the MCP boundary; [ORB-10271] implemented private spoke self-registration from validated `host.toml`, staged projection results, and definitive-success cache refresh.

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

**Partially retained.** The `machine_id` contract, the names-resolve-at-binding
rule, and the out-of-band pinning of a route target all survive verbatim; v1 only
generalizes "the one hub" to "each route." The *registry* half — the hub-side
inventory, name reservation across the fleet, and tombstone aliases — is deferred
with [ADR-0358], which means v1 has no cross-machine name reservation at all.

## ADR-0228 — Local placement broker with capability-set filtering

**Status:** Accepted · 2026-07 · [ORB-10245] froze tool routing and authorization; [ORB-10267] added the `orbit.host.list` and `orbit.workspace.list` canonical discovery tools (hub placement, operator capability, workspace-unscoped) reading one sanitized path-free registry snapshot; [ORB-10268] implemented the fixed checkoutless hub endpoint and exact scalar-capability surface; [ORB-10271] enforced current registered/active caller identity before every ordinary remote call and exposed path-free operator friction list/show; [ORB-10319] colocates MCP-only discovery schema and execution in Remote while retaining dedicated human CLI commands; [ORB-10276] added the workspace-scoped, `{agent, operator}` `orbit.crew.list` (never `runner`) beside the two operator-only global registry tools, reading the owner execution-profile projection through one service that also backs explicit task-crew validation; [ORB-10332] removed the `orbit.host.list` MCP discovery tool as unused (the `orbit host list` CLI command and the `orbit.workspace.list` / `orbit.crew.list` MCP tools remain).

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

**Amended by [ADR-0355] and [ADR-0358].** The broker and its four placement classes
survive; `hub` is reinterpreted from "the one main host" to "the coordinating
machine for this tool's workspace," which for an owned workspace is in-process. The
`runner` capability set is withdrawn with execution placement, and the
`orbit.workspace.list` / `orbit.crew.list` discovery tools now read local registry
data rather than a fleet snapshot.

## ADR-0229 — Owner-authored knowledge with hub-global IDs and explicit replicas

**Status:** Superseded by [ADR-0357] · 2026-08 · originally accepted 2026-07 — [ORB-10245] fixed the one-writer knowledge rule; [ORB-10272] implemented its dormant hub-global sequence, validated reconciliation, and immutable allocation-ledger substrate without activating public issuance; [ORB-10330] added the owner-side preallocated finalizers and the gated broker composition (hub allocation → exact-owner finalization) while keeping public creation on the compatibility path until F3.

### Context

Knowledge needs global IDs without making a hub checkout or a stale replica a second
author.

### Decision

The hub allocates global IDs, the declared owner authors current knowledge, and Git
replicas are opt-in reads marked as replicas. The hub never proxies to a spoke owner.
Hub activation first reconciles every registered workspace's complete hub-local
legacy file/allocation inventory; missing sources and cross-workspace duplicate IDs
fail before mutation. A later workspace stays knowledge-ineligible until the same
reconciliation succeeds under the allocator lock.

### Consequences

- A non-owner agent routes actionable work as a task to the owner.
- Exact `mcp_call_id` replay is idempotent only for the same full request identity;
  sequence advance, immutable ledger append, and canonical hub audit commit atomically.
- Standalone/worktree allocation remains the compatibility path until F3 performs
  the explicit activation and caller cutover.
- Cost: finalize failure consumes a valid unused ID, and current spoke-owned knowledge is unavailable off-owner.

**Superseded because the global ID was never needed.** All of the machinery above —
reconciliation, the allocation ledger, the dormant/active marker, the
allocate-then-finalize composition — exists to make one number unique across
machines. [ADR-0357] observes that knowledge records are already addressed within a
workspace and keys them `(workspace_id, artifact_key)`, at which point the entire
protocol evaporates: no reservation, no expiry, no orphaned ID, no finalize/pull
race. F3 never ran, so no ID was ever issued from the hub sequence and nothing needs
renumbering. The one-writer-per-workspace rule this ADR established survives intact.

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
and [ADR-0358] does not replace it with anything — v1 simply has no case for
running a task anywhere but the machine that owns its workspace. The mailbox
posture and the immutable requested/actual snapshot are the parts to reread if
placement returns ([3_vision.md §1 Q3](./3_vision.md)).

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
against local `host.toml` and locally-known owner names only, so an unrecognized
name cannot be distinguished from a typo and there is no `last_seen` to notice a
quiet owner. The own-host case — the one that decides whether anything fires — stays
decidable offline, which was always the load-bearing half.

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

**Replacement:** [ADR-0240] superseded this horizontal boundary when [ORB-10319]
landed. ADR-0235 remains the accepted historical decision that explains the
intermediate architecture.

## ADR-0240 — Consolidate remote coordination in one vertical feature crate

**Status:** Accepted · 2026-07 · [ORB-10319] implemented the boundary.

### Context

The registry-only extraction in ADR-0235 still made one remote feature cross CLI,
Core, Registry, Store, MCP, and Tools whenever persistence, contract composition,
and routing changed together. A separate broker crate would add another
horizontal layer while leaving that coordination cost in place.

### Decision

Rename and widen `orbit-registry` into `orbit-remote`, a vertical feature crate that
owns host/workspace registry behavior, its SQLite statements and feature schema,
profiles and caches, MCP contract composition, the local broker, hub authority and
link, and registration. Keep `orbit-store` and `orbit-mcp` as neutral kernels,
shared DTOs in `orbit-common`, generic builtin definitions in `orbit-tools`, and the
transport-independent coordination executor in `orbit-core`. Reuse the same
config-resolved `orbit.db`; do not introduce a Remote database or a separate broker
crate.

### Consequences

- Remote behavior and persistence evolve behind one crate API without reverse
  dependencies from Core, Store, MCP, Tools, or Common.
- Remote v1 adopts the existing global v5/v6/v8 registry tables in place through
  Store's namespaced feature-migration ledger, preserving every row.
- Remote v2 adds the dormant hub-global knowledge sequence and reconciliation
  tables in the same database [ORB-10272]; the feature can evolve that transaction
  without moving knowledge policy into Store.
- CLI and dashboard remain thin consumers; generic MCP framing and raw client code
  no longer know registry, graph, learning, hub, or placement policy.
- Cost: `orbit-remote` is intentionally broad and requires internal module
  discipline; changes shared by unrelated features still belong in a neutral
  kernel and must not be absorbed merely for convenience.

## ADR-0352 — Gate workflow dispatch on an exclusive TTL'd workspace claim

**Status:** Accepted · 2026-08 · [ORB-10709]

### Context

Ownership binds a workspace to a machine, not to an operator, and several operator
sessions can now reach one workspace concurrently. The duplicate-dispatch guard is
keyed on task id over a bounded window, and discovery-mode submissions carry no
task ids at all, so two of them in one workspace both proceed. File reservations
arbitrate between workers, not between orchestrators.

### Decision

Take an exclusive, TTL-bounded workspace claim as a precondition for the governed
workflow operations only, enforced at the shared run-submission path so every
surface inherits it. Acquisition mints a holder-presented claim token; contention
rejects with holder and expiry; force-release exists and is audited.

### Consequences

- Concurrent dispatch becomes impossible by construction, including on the
  currently unguarded discovery path, while filing and inspecting work stay
  concurrent so several people can work different features in one workspace.
- Cost: a third TTL-bounded exclusive hold joins reservations and leases, at a
  third granularity, with no self-evident distinction in the names.
- Cost: a dead holder blocks dispatch until TTL; force-release is both the escape
  hatch and the thing that makes the guarantee advisory if it becomes habitual.
- Cost: the claim token is client-held state, and a holder that loses it must wait
  or force.
- Cost: contradicts [resident-orchestrator/2_design.md §3](../resident-orchestrator/2_design.md),
  which avoided a lease subsystem deliberately; that reasoning is revised, not
  silently overridden.

**As implemented ([ORB-10709]).** Three choices the decision above left open,
recorded here because each is load-bearing:

- **The claim reuses the `task_reservations` table**, separated from worker file
  reservations by a `scope` discriminator (`files` / `workspace_claim`, schema
  migration v13) rather than by a parallel table. That inherits the atomic
  `IMMEDIATE`-transaction acquisition, TTL, lazy expiry, audit, and release
  escape hatch already built there. *Rejected alternative:* a dedicated
  `workspace_claims` table — honest about the "distinct dimension", but roughly
  250 lines duplicating the machinery this decision exists to reuse. The scope
  discriminator is a required argument of the shared SQL predicate, so the
  compiler asks every query site which dimension it reads; claim rows
  additionally carry an empty file list, so even a forgotten filter cannot
  produce a path conflict.
- **An unclaimed workspace gates nothing.** The claim arbitrates *between*
  operators who want one; it is not a mandatory ceremony before every dispatch.
  Requiring one unconditionally would break every existing unattended dispatch
  and make "naming the current holder" in a refusal meaningless. Refusal happens
  only when an active claim exists and the caller presents no token or a stale
  one.
- **`orbit run ship-sweep` stands down rather than failing** when it meets a held
  claim: the unattended sweep has no token and does not force, so a claimed
  workspace is reported as skipped with the holder, not as a sweep error.

Surfaces: `claim_token` on the `orbit.workflow.ship` / `orbit.workflow.run.resume`
tools, `orbit run ship --claim-token`, the dashboard ship and resume bodies, and
`ORBIT_WORKSPACE_CLAIM_TOKEN` for an operator shell. Acquire / release /
force-release / status are the `orbit.workspace.claim.*` tools, registered
inactive alongside `orbit.task.locks.*` — operator-reachable through `orbit tool
run`, absent from the agent MCP surface. `orbit.workspace.claim.release` is a
governed operation requiring `Operator`, because force-release displaces whoever
is driving dispatch.

**Unaffected by the v1 revision.** The claim answers "which operator is driving
this workspace right now"; ownership answers "which machine holds it." [ADR-0355]
changes the second and leaves the first intact — collapsing them would reintroduce
exactly the split-brain [ADR-0200] rejected.

## ADR-0355 — Every machine is its own coordination host

**Status:** Accepted · 2026-08 · supersedes [ADR-0226]
**Scope:** coordination-record routing, workspace ownership, machine roles

### Context

[ADR-0226] fixed the coordination plane at one main host for every workspace, with
each machine declaring `mode = standalone | hub | spoke` at init. Because `mode` is
machine-level, a machine that becomes a spoke routes *every* workspace on it to the
hub — including projects that exist only on that machine. There was no per-workspace
opt-out, and opting out of the hub entirely meant giving up unique task IDs, since
uniqueness was a property of the hub's single allocator.

The invariant the hub actually protected is *one coordination writer per workspace*.
"One writer globally" is strictly stronger, and the extra strength is what forced
local projects through a machine they had no reason to touch.

### Decision

A workspace's coordination records live in the store of the machine that owns it.
Every machine is a coordination host for the workspaces it owns and for no others.
Remove `mode` from `host.toml`; a machine's role is derived per workspace from its
own registry, and the all-owned case needs no declaration. Ownership stays declared
in the machine-local workspace registry, which is the v1 source of truth.

Coordination writes in a checkout the machine does not own are refused at the shared
run-submission chokepoint — not merely hidden from reads — with the owner named in
the error. Replica checkouts stay in the local registry so path resolution and the
local-derived tier keep working; they are filtered from `orbit workspace list`
rather than removed.

### Consequences

- A purely local project stays local with no configuration and no remote dependency.
- One machine can coordinate some workspaces and hold read-only checkouts of others,
  which the superseded model could not express.
- The [ORB-10248] catalog/checkout split already carries `owner_machine_id` and the
  `owner`/`replica` role, so the substrate needs no change — only the layer above it
  is removed.
- Cost: every owner becomes a single point of failure for its own workspaces. This
  is better for blast radius and worse for predictability than one hub — you now
  have several machines that can each take part of the system offline, and which
  ones depends on a per-machine file.
- Cost: "is this workspace here?" has three correct answers in a replica checkout
  (present on disk, absent from `list`, refused for writes), and only the error
  message explains it.

## ADR-0356 — Machine-scoped task-id prefix instead of a global allocator

**Status:** Accepted · 2026-08 · [ORB-10723] implemented prefix- and width-agnostic parsing, host-prefix minting, and registry-constrained text scanning; [ORB-10728] implemented foreign-prefix relation storage and local-only validation.
**Scope:** task ID minting, ID parsing, cross-machine references
**Code anchors:** `crates/orbit-common/src/types/task_artifacts.rs`, `crates/orbit-common/src/types/task.rs::task_reference_is_not_verifiable_here`, `crates/orbit-store/src/sqlite/task_registry/store.rs::parse_orb_task_number`, `crates/orbit-core/src/command/docs/artifact_ref.rs`

### Context

Global task-ID uniqueness was bought with a global allocator: one authority, one
sequence, and therefore one machine every project had to reach. The alternative was
never evaluated, because the first design treated uniqueness as an authority
problem.

### Decision

Give each machine a `task_prefix` chosen once at global init and immutable
thereafter. Every task that machine mints is `<task_prefix>-NNNNN` against its own
monotonic sequence. Uniqueness across machines follows from the prefixes differing —
a human-scale choice, not a coordinated allocation. Reject `ORB`, `ADR`, `L`, and
`F` as prefixes; they are live artifact-reference namespaces. Existing installs keep
`ORB`, so no existing ID or citation changes.

### Consequences

- No allocator, no reconciliation, no activation transition, and no machine that
  must be reachable to file a task.
- Divergent ownership becomes recoverable: two machines that both believe they own a
  workspace produce disjoint task sets that merge by union with nothing to renumber.
  This is what makes deferring registration ([ADR-0358]) safe, so the two decisions
  hold each other up and neither should be adopted alone.
- Moving a workspace between owners becomes a row copy rather than an ID rewrite.
- An unresolved target under a locally known prefix remains invalid. An unresolved
  foreign-prefix target is stored and rendered as `not verifiable here`; a foreign
  dependency is non-gating because this machine cannot observe its lifecycle state.
- Cost: prefix collisions are possible and silent. Nothing detects two machines
  choosing the same prefix until their records meet, and v1 adds no lint.
- Cost: cross-machine chronology is destroyed. `ORB-10601` was visibly later than
  `ORB-10248`; per-machine sequences do not compare, and readers who have
  internalized otherwise will be wrong without being told.
- Cost: ID parsing must become prefix- and width-agnostic across four call sites,
  which makes the text scanner weaker against prose — it must match only prefixes
  known to the local registry rather than any `[A-Z]{2,5}-[0-9]+` shape.

## ADR-0357 — Workspace-scoped knowledge keys, no global knowledge IDs

**Status:** Accepted · 2026-08 · supersedes [ADR-0229]
**Scope:** learnings, ADRs, frictions; any cross-workspace read surface

### Context

Learnings and ADRs were assigned hub-global IDs, which required the dormant sequence
service, a validated reconciliation of every registered workspace, an immutable
allocation ledger, and an allocate-then-finalize composition careful enough to
survive a crash between the two steps ([ORB-10272], [ORB-10330]). All of it exists
to make one number unique across machines. Frictions and run IDs, meanwhile, were
already per-workspace and had caused no trouble.

### Decision

Key learnings, ADRs, and frictions by `(workspace_id, artifact_key)`. No record type
has a global allocator. Remove the [ORB-10272] substrate rather than parking it:
public issuance never activated, so no ID was ever allocated from it, and it encodes
a superseded model.

Because IDs now collide across workspaces by design, any merged cross-workspace read
surface **must** carry the `workspace` field. A bare ID from a merged search is not
addressable.

### Consequences

- The reservation/finalization protocol evaporates — no expiry, no orphaned IDs, no
  finalize/pull race, because there is no allocation step.
- Owner-only authorship (the rule [ADR-0229] established) survives unchanged, and
  now needs no protocol to enforce it.
- Together with the retirement of the ADR store
  ([CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict)), the last consumer of
  hub-allocated knowledge IDs disappears.
- Cost: `ADR-0234` means different things in different workspaces. Cross-repo
  citation is now forbidden rather than merely discouraged, and the existing
  merged-search footgun — an ID that silently addresses the wrong record — widens
  from frictions to every knowledge type. Fixing the search projection is a
  precondition of this change, not a follow-up.

## ADR-0358 — Defer fleet registration and execution placement to v2

**Status:** Accepted · 2026-08 · supersedes [ADR-0230]; defers parts of [ADR-0227], [ADR-0228]; amends [ADR-0231]
**Scope:** multi-machine scope boundary; what v1 refuses to build

### Context

The superseded model shipped, or specified, a substantial fleet layer: host
registration and inventory, the workspace presence map, pull-based run leases, a
`runner` capability, and poll-driven liveness. Its only v1 consumer would be
convenience. The cross-machine capability actually wanted is narrower — create and
read tasks in a workspace owned by another machine — and the client→owner SSH-carried
MCP path already provides it.

### Decision

v1 has no fleet inventory, no registration protocol, no presence map, no execution
placement, no run leases, and no `runner` capability. Each machine's
`workspaces.json` is the source of truth for what it owns, self-asserted and
unarbitrated. Runs execute on the workspace's owner, in-process. The entire
cross-machine surface is task coordination over an existing route.

The shipped registry core, presence projection, execution-profile CAS, and registry
cache are **retained dormant** for v2, unreachable from any v1 path. [ORB-10272]'s
knowledge-allocation substrate is deleted instead, because unlike the registry it
contradicts rather than merely anticipates the new model.

### Decision boundary

Deferring registration is only safe because [ADR-0356] makes competing ownership
claims produce disjoint records. Adopting this deferral without the prefix change
would leave a silent, unrecoverable ID collision.

### Consequences

- Adding a machine is: init with a prefix, adopt or link workspaces, add a route.
  No registration, no credential a satellite must hold standing.
- The star topology is gone; a machine initiates to the owners it holds replicas of,
  and nothing initiates back. Dispatching work *to* a machine is therefore
  impossible in v1 by construction, not merely unimplemented.
- Cost: ownership conflicts are undetectable until a human notices, and there is no
  cross-machine name reservation, so tombstone aliases do not exist in v1 and a
  rename strands human-authored text naming the old name.
- Cost: routine pin validation weakens to local data, unable to distinguish a typo
  from an unknown machine, with no `last_seen` to flag a quiet owner.
- Cost: carefully built and reviewed work ([ORB-10268], [ORB-10269], [ORB-10271])
  becomes dormant. "Deferred" is a promise that costs maintenance and may never be
  collected.

## Task References

- [ORB-00424] — completed design proposal for canonical Orbit MCP and Bridge parity retirement.
- [ORB-10245] — accepted the coupled contract and recorded this ADR set.
- [ORB-10709] — shipped the exclusive TTL'd workspace claim and gated workflow
  dispatch on it at the shared run-submission path (ADR-0352).
- [ORB-10723] — implemented ADR-0356: task parsing accepts registered machine
  prefixes and wider numeric suffixes, while allocation retains one monotonic
  machine sequence and formats it with the immutable host prefix.
- [ORB-10728] — stores unresolved foreign-prefix task relations with an explicit
  `not verifiable here` projection, keeps locally prefixed misses as hard errors,
  and treats foreign dependencies as non-gating for local readiness.
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
  `role` (owner/replica) operations, the `orbit.host.list` (removed in [ORB-10332]) and
  `orbit.workspace.list` canonical discovery tools, one path-free `RegistrySnapshotV1`
  projection read in a single store
  transaction, typed `workspace-required|global` tool scope metadata, the versioned atomic
  satellite registry cache (stable canonical payload plus local receipt; derived age/freshness
  does not create same-revision conflicts), and store schema v8's singleton
  `hub_registry_metadata` row carrying the hub `machine_id` and the monotonic
  `registry_revision` (advanced once per snapshot-visible mutation; no-ops do not advance it).
  Current-machine rename holds one machine-global lock across exact preflight, a staged,
  reparsed `host.toml` write, the durable registry rename, and post-error outcome
  classification. `machine_id` uses the canonical path-free `hm_` namespace, and hub-local
  CLI enumeration renders the same single-transaction sanitized snapshot as discovery.
  Poll-driven refresh remains deferred to I/J.
- [ORB-10268] — implemented Unit E1's strict machine-global hub trust document and
  fixed checkoutless hub MCP endpoint, including repeated exact store-stamp checks,
  stable workspace-only payloads, canonical placement/capability filtering, and
  trusted one-call/one-audit provenance.
- [ORB-10269] — implemented Unit E2's negotiated bounded SSH-carried hub link,
  scalar-capability peer reuse, trusted remote session metadata, and no-replay
  transport error classification.
- [ORB-10271] — implemented Unit E3's connector-private registration, honest
  committed-stage results, local cache refresh boundary, ordinary active-caller
  guard, path-free coordination frames, friction reads, and two-root canary.
- [ORB-10302] — implemented Unit C4: moved the host/workspace/cache/service domain
  and its tests into `orbit-registry`, retired the replicated-registry scaffold,
  and preserved core/store/common ownership boundaries ([ADR-0235]).
- [ORB-10319] — implements the proposed ADR-0240 boundary by renaming and widening
  the registry crate into vertical `orbit-remote`, adopting its persistence in
  place, moving profile/routine/MCP composition into the feature, and removing
  Registry/Remote dependencies from Core and MCP.
- [ORB-10272] — implements ADR-0229's dormant allocation substrate in Remote v2:
  full pre-mutation legacy reconciliation, independent forward-only ADR/learning
  sequences, immutable correlation ledger plus atomic audit, replay-safe lookup,
  and late-workspace ineligibility. F3 retains authority over public activation and
  caller cutover; standalone creation remains unchanged.
- [ORB-10276] — implemented Unit H1 of ORB-10246: completed host/workspace/crew
  discovery with the workspace-scoped, `{agent, operator}`, hub-placement
  `orbit.crew.list` and added the single `orbit-remote` execution-profile projection
  service (injected clock, one shared freshness TTL) backing both crew discovery and
  explicit task-crew validation on task add/update. It consumes C2's stored owner
  projection only — never hub-local crews, the satellite cache, a stale replica, or a
  synchronous owner call — and returns a `ValidatedCrewProfile` (stored profile,
  resolved crew, generation, config digest, ship-closure digest). Standalone and
  owner-local auto-task CRUD keep their local crew validation. Task `host`/claims
  (H2), workflow ship/placement (H3), and run lineage/leasing (I1) stay out of scope.
- [ORB-10332] — removed the `orbit.host.list` MCP discovery tool as unused; the
  `orbit host list` CLI command and the `orbit.workspace.list` / `orbit.crew.list`
  MCP discovery tools remain.
