---
title: Host Registry — Decisions
owner: claude
last_updated: 2026-08-12
last_validated: 2026-08-11
status: Draft
feature: host-registry
doc_role: decisions
type: design
summary: Decision record for host identity, per-machine coordination, prefix-partitioned task IDs, workspace-scoped knowledge keys, and the deferral of fleet registration and execution placement.
tags: [host-registry, mcp-bridge, multi-host, ownership]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**"]
related_features: [host-registry, mcp-bridge]
related_artifacts: [ORB-00424, ORB-10245, ORB-10248, ORB-10249, ORB-10255, ORB-10257, ORB-10267, ORB-10258, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10276, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ORB-10709, ORB-10723, ORB-10728, ORB-10730, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0352, ADR-0355, ADR-0356, ADR-0357, ADR-0358]
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

**Status:** Accepted · 2026-07-17 07:03:25.976551Z · [ORB-10245]
**Owner:** codex
**Created:** 2026-07-17 07:02:53.852664Z
**Last updated:** 2026-07-17 07:03:25.976551+00:00
**Related features:** `host-registry`, `mcp-bridge`
**Tags:** `coordination`, `ownership`, `placement`, `v1`
**Paths:** `docs/design/host-registry/**`, `docs/design/mcp-bridge/**`

### Context
Cross-machine work needs a coordination authority, a knowledge author, and an execution destination; treating them as the same authority would scatter task state or make ownership implicit.
### Decision
Use exactly one coordination hub for every workspace, declare one workspace owner, and select execution placement per run with the owner as the default.
### Consequences
- Coordination writes remain hub-routed while knowledge authorship remains owner-bound.
- Cost: hub downtime stalls coordination for every workspace, and disconnected machines cannot write coordination records.

## ADR-0227 — Stable machine identity, registry, and out-of-band hub pin

**Status:** Accepted · 2026-07-17 07:03:26.175980Z · [ORB-10245], [ORB-10255]
**Owner:** codex
**Created:** 2026-07-17 07:03:20.499802Z
**Last updated:** 2026-07-18 07:04:05.364678Z
**Related features:** `host-registry`, `mcp-bridge`
**Tags:** `identity`, `registry`, `transport-trust`, `v1`
**Paths:** `docs/design/host-registry/**`, `docs/design/mcp-bridge/**`

### Context
Hostname-derived strings and per-workspace transport targets can silently redirect a machine or elevate repository configuration.
### Decision
Assign every machine an immutable generated machine_id, keep the registry at the hub, and pin the one hub machine_id out of band in machine-local mcp.toml.
### Consequences
- Names resolve once at binding time and persisted records retain the stable identity.
- Cost: bootstrap transfers the hub identity out of band and registry/trust drift requires explicit diagnosis.

## ADR-0228 — Local placement broker with capability-set filtering

**Status:** Accepted · 2026-07-17 07:03:26.373231Z · [ORB-10245]
**Owner:** codex
**Created:** 2026-07-17 07:03:20.635772Z
**Last updated:** 2026-07-17 07:03:26.373231Z
**Related features:** `mcp-bridge`, `host-registry`
**Tags:** `broker`, `placement`, `capabilities`, `v1`
**Paths:** `docs/design/mcp-bridge/**`

### Context
A single remote MCP target must preserve local graph/doc behavior and must not equate tool placement with caller privilege.
### Decision
Make the client-facing MCP process a local placement broker whose canonical tools declare hub, owner, local-derived, or composite placement and whose effective capability set is independently filtered.
### Consequences
- Tool schemas have one placement and a non-empty allowed capability set.
- Cost: the broker owns route preflight, composite auditing, and capability-by-placement conformance coverage.

## ADR-0229 — Owner-authored knowledge with hub-global IDs and explicit replicas

**Status:** Accepted · 2026-07-17 07:03:26.566162Z · [ORB-10245]
**Owner:** codex
**Created:** 2026-07-17 07:03:20.765693Z
**Last updated:** 2026-07-17 07:03:26.566162Z
**Related features:** `host-registry`, `mcp-bridge`
**Tags:** `knowledge`, `ownership`, `replicas`, `ids`, `v1`
**Paths:** `docs/design/host-registry/**`, `docs/design/mcp-bridge/**`

### Context
Knowledge needs globally allocated identifiers without making a hub checkout or a stale replica a second author.
### Decision
The hub allocates global IDs, the declared owner authors current knowledge, and Git replicas are opt-in reads marked as replicas; the hub never proxies to a spoke owner.
### Consequences
- A non-owner agent files work for the owner rather than writing through a new route.
- Cost: finalize failures consume valid-but-unused IDs and current spoke-owned knowledge is unavailable off-owner.

## ADR-0230 — Pull-based leases with immutable placement and explicit recovery

**Status:** Accepted · 2026-07-17 07:03:26.764043Z · [ORB-10245]
**Owner:** codex
**Created:** 2026-07-17 07:03:20.898467Z
**Last updated:** 2026-07-17 07:03:26.764043Z
**Related features:** `host-registry`, `mcp-bridge`
**Tags:** `leases`, `placement`, `recovery`, `runner`, `v1`
**Paths:** `docs/design/host-registry/**`, `docs/design/mcp-bridge/**`

### Context
A hub-push executor model would require outbound spoke routing and makes retries obscure the placement actually selected and leased.
### Decision
Spokes poll the hub for placed runs; requested and actual placement are immutable, pre-start loss returns a run for redelivery, and post-start uncertainty requires explicit recovery.
### Consequences
- The hub is a mailbox and never opens a route to a spoke.
- Cost: pickup latency follows poll cadence and an interrupted started run requires operator/shepherd recovery rather than silent reassignment.

## ADR-0231 — Committed-routine ownership with host-local cursors

**Status:** Accepted · 2026-07-17 07:03:26.898178Z · [ORB-10245], [ORB-10258], [ORB-10270]
**Owner:** codex
**Created:** 2026-07-17 07:03:21.036752Z
**Last updated:** 2026-07-18 23:03:06.740466Z
**Related features:** `host-registry`
**Tags:** `routines`, `ownership`, `cursor`, `v1`
**Paths:** `docs/design/host-registry/**`, `docs/design/routines/**`

### Context
Git-committed routines converge to many checkouts, while scheduler cursors and pauses must remain locally meaningful.
### Decision
A committed routine is owned by its registry-validated host pin; unpinned committed routines fail closed, and each host retains its own cursor and pause state.
### Consequences
- Reassigning a routine is a reviewed pin change rather than a git-status inference.
- Cost: handoff starts with no migrated cursor and existing committed routines need explicit pins before enforcement.

## ADR-0232 — Retire Bridge’s Orbit-shaped contract

**Status:** Accepted · 2026-07-17 07:03:27.035875Z · [ORB-10245]
**Owner:** codex
**Created:** 2026-07-17 07:03:21.165568Z
**Last updated:** 2026-07-17 07:03:27.035875Z
**Related features:** `mcp-bridge`
**Tags:** `bridge`, `contract`, `retirement`, `v1`
**Paths:** `docs/design/mcp-bridge/**`

### Context
Bridge parity duplicated Orbit schemas, errors, and workflow declarations despite Orbit being the canonical domain owner.
### Decision
Retire Bridge’s Orbit-shaped contract after Orbit MCP reaches parity; Bridge remains only for its non-Orbit constellation domains.
### Consequences
- Clients register Orbit and Bridge side by side during migration.
- Cost: cutover temporarily maintains two client registrations and requires deleting a compatibility layer rather than extending it.

## ADR-0235 — Make orbit-registry the singular host/workspace registry domain crate

**Status:** Accepted · 2026-07-18 18:58:42.207312Z · [ORB-10302]
**Owner:** codex
**Created:** 2026-07-18 18:58:37.266035Z
**Last updated:** 2026-07-18 18:58:42.207312Z
**Related features:** `host-registry`, `mcp-bridge`
**Tags:** `crate-boundary`, `host-registry`, `singular-hub`
**Paths:** `crates/orbit-registry/**`, `crates/orbit-core/src/host_registry.rs`, `crates/orbit-core/src/routines/host.rs`, `crates/orbit-core/src/workspace_registry.rs`, `crates/orbit-core/src/registry_cache.rs`, `crates/orbit-store/**`

### Context

C3 left strict host identity, the logical workspace catalog, the satellite cache, and the store-backed registry service in orbit-core while the existing orbit-registry crate exposed an unused opaque-byte replication and merge model. The real alternatives were to keep that domain in orbit-core, retain a generic replication substrate alongside it, or establish a dedicated one-way registry domain boundary.

### Decision

Repurpose orbit-registry as the machine/workspace registry domain crate. It owns host identity, local workspace catalog/checkouts/roles, registry-cache semantics, and the store-backed HostRegistryService; orbit-core depends on it and temporarily re-exports compatibility surfaces. orbit-store remains the only owner of SQL, migrations, revision advancement, and transactional snapshot queries, and it must never depend on orbit-registry. The opaque replicated Registry/Replica/merge/transport model is retired because v1 has one singular coordination hub and no replicated registry writers.

### Consequences

- The intended dependency direction is orbit-core -> orbit-registry -> orbit-store -> orbit-common, with orbit-core also retaining its direct orbit-store edge.
- Runtime execution-profile construction, catalog validation, and ship-closure hashing stay in orbit-core; shared DTOs stay in orbit-common.
- Compatibility re-exports let current callers migrate imports incrementally without preserving a second domain implementation.
- Cost: orbit-registry is no longer a consumer-agnostic leaf and now compiles the store layer; reversing this boundary would require moving the domain again or introducing a cycle-prone abstraction.

## ADR-0240 — Consolidate remote host and MCP behavior in the vertical orbit-remote crate

**Status:** Accepted · 2026-07-19 07:19:02.142649Z · [ORB-10319]
**Owner:** codex
**Created:** 2026-07-19 03:09:37.970409Z
**Last updated:** 2026-07-19 07:19:02.142649Z
**Related features:** `host-registry`, `mcp-bridge`
**Supersedes:** `ADR-0235`
**Tags:** `architecture`, `crate-boundary`, `orbit-remote`, `vertical-feature`, `plugin-style`, `dependency-direction`
**Paths:** `crates/orbit-remote/**`, `crates/orbit-cli/src/command/mcp/**`, `crates/orbit-core/**`, `crates/orbit-store/**`, `crates/orbit-tools/**`, `crates/orbit-mcp/**`, `crates/orbit-common/**`, `crates/orbit-dashboard/**`, `ARCHITECTURE.md`, `scripts/check-dependency-direction.sh`, `docs/design/host-registry/**`, `docs/design/mcp-bridge/**`

### Context

The coupled Host Registry and MCP Bridge implementation now spans registry identity/catalog/cache in `orbit-registry`, active registry SQL in `orbit-store`, registry-aware runtime and tool dispatch in `orbit-core`, canonical remote tools in `orbit-tools`, remote protocol hooks in `orbit-mcp`, and broker/hub/link/registration composition in `orbit-cli`. The previously proposed answer was an additional horizontal `orbit-mcp-broker` crate above those layers. That would remove code from the CLI but preserve the seven-crate change tax for every later remote feature.

The real alternatives are: keep the horizontal layers and add the broker crate; absorb all RMCP infrastructure into the feature; or create one vertical remote feature crate while retaining small generic infrastructure kernels. The first preserves the coupling problem, while the second makes reusable RMCP transport inseparable from Orbit's host-registry policy.

### Decision

Rename and expand `orbit-registry` into one internal vertical feature crate, `orbit-remote`. It owns host identity, workspace roles/catalog/cache, registry services, active registry SQL and namespaced feature migrations, trusted hub configuration, broker/hub/link/registration composition, remote tool definitions and handlers, remote-specific MCP contracts and client behavior, and local graph/learning MCP composition.

The dependency direction is `orbit-cli|orbit-dashboard -> orbit-remote -> {orbit-core, orbit-store, orbit-tools, orbit-mcp, orbit-graph, orbit-graph-extract, orbit-common}`. Core, Store, Tools, MCP, and Graph do not depend back on Remote. Core exposes registry-free runtime bindings, execution-environment snapshots, routine-placement projections, and the transport-independent checkoutless `HubCoordinationExecutor`; Remote calls that executor from its hub/broker orchestration without injecting remote tools into Core. Store exposes generic pooled-read, transaction, and namespaced feature-migration capabilities. Tools retains its generic builtin registry, while Remote composes those definitions with feature-owned discovery and graph definitions. MCP retains only reusable RMCP framing, schema/name translation, stdio transport, raw client primitives, and generic extension/composition hooks.

Keep the existing config-resolved global `orbit.db`. Shipped global migrations v5, v6, and v8 remain immutable compatibility shims and v7 remains Store-owned audit schema. Global v9 introduces only the generic feature-schema ledger; `orbit-remote` feature migration v1 validates and adopts the existing registry tables without copying, renaming, or rewriting data. All future remote schema changes advance the Remote feature ledger rather than editing Store's domain migration list.

This decision supersedes ADR-0235's narrower `orbit-registry -> orbit-store` domain boundary and replaces the unimplemented `orbit-mcp-broker` proposal.

### Consequences

- Later registry, knowledge-routing, placement, runner, and hub-link work has one owning feature crate; shared crates change only when a genuinely generic contract or infrastructure seam changes.
- `orbit-cli` and `orbit-dashboard` become consumers of a stable Remote facade instead of composition owners, and `orbit-core` no longer imports registry types or services.
- Registry persistence keeps its existing database, table names, transaction boundaries, and data while active SQL and behavior tests move beside the domain.
- `orbit-mcp` remains reusable and acyclic; graph, learning, hub contract, registration, and routing policy are composed by Remote.
- Historical migrations cannot be deleted after ownership moves; Store permanently retains the frozen bootstrap shims required to open old and fresh databases.
- Cost: this is a larger atomic refactor than a broker-only move. It requires neutral Core factories/providers, one generic Store migration seam, MCP composition hooks, and coordinated test relocation before the crate rename can compile without a dependency cycle.

## ADR-0352 — Gate workflow dispatch on an exclusive TTL'd workspace claim

**Status:** Accepted · 2026-08-10 03:19:10.679145Z · [ORB-10709]
**Owner:** human
**Created:** 2026-08-10 01:29:41.181497Z
**Last updated:** 2026-08-10 03:19:10.679145Z
**Related features:** `host-registry`
**Tags:** `host-registry`, `workflow`, `capability`, `coordination`
**Paths:** `crates/orbit-core/src/command/job/pipeline.rs`, `crates/orbit-core/src/runtime/workspace_claim.rs`, `crates/orbit-common/src/authorization.rs`, `crates/orbit-store/src/sqlite/**`, `crates/orbit-core/src/runtime/task_locks.rs`

### Context

Workspace ownership is a declared binding to a machine, never a runtime claim, and the design states the reasoning plainly: coordination has one writer by construction, and two owners for one workspace is split-brain. That held while one operator drove a workspace.

It no longer holds. Several operator sessions can now reach the same workspace concurrently — an off-box orchestrator over the owned tunnel, a local operator broker, a session over SSH — and nothing arbitrates between them. Ownership answers *which machine*, not *which operator*, and two operator sessions on one machine are indistinguishable to the existing model.

The guards that exist do not cover this. The duplicate-dispatch guard is per task id and scans a bounded window of recent runs, so a stale non-terminal run outside the window is invisible. Worse, auto and backlog-discovery submissions carry no task ids at all and are unguarded entirely: two discovery ship runs in one workspace both proceed. Task reservations are file-scoped, advisory, and enforced only at gate-pipeline admission — they arbitrate between workers, not between orchestrators.

The contention is specific. Reading, searching, filing tasks, and authoring knowledge are safe concurrently, and several people working different features in one workspace is the desired behaviour. What cannot be concurrent is *dispatch*: triage, ship, and resume decide what work starts and against which base, and two orchestrators making those decisions independently produce duplicated runs and racing branch state.

### Decision

Introduce an exclusive, TTL'd **workspace claim** held by one operator, and make it a precondition for workflow dispatch only.

- The claim gates exactly the governed workflow operations. Every other operation — task create, read, update, search, knowledge, friction — is unaffected and remains concurrent.
- Enforcement lives at the shared run-submission path, not at a protocol adapter, so every surface inherits it: CLI, HTTP, MCP, and remote command execution alike. A caller holding shell cannot route around it, because the CLI reaches the same chokepoint.
- Acquisition mints a **claim token** returned to the holder and presented on subsequent workflow calls. Machine and session identity are recorded for diagnostics but are not load-bearing, because session identity is minted per connection and does not survive a reconnect.
- Contention is a rejection carrying the current holder and the expiry instant, never a silent queue or a silent steal.
- The claim is TTL-bounded with lazy expiry evaluated on each check, and an explicit force-release exists and is audited.
- Claim scope is a distinct dimension from file reservations. It must not be expressed as a whole-workspace file selector, which would also block the worker reservations it is meant to leave alone.

## As implemented (ORB-10709)

Three choices the decision above left open, each load-bearing:

- **The claim reuses the `task_reservations` table**, separated from worker file reservations by a `scope` discriminator (`files` / `workspace_claim`, schema migration v13) rather than by a parallel table. That inherits the atomic `IMMEDIATE`-transaction acquisition, TTL, lazy expiry, audit, and release escape hatch already built there. *Rejected alternative:* a dedicated `workspace_claims` table — honest about the "distinct dimension", but roughly 250 lines duplicating the machinery this decision exists to reuse. The scope discriminator is a required argument of the shared SQL predicate, so the compiler asks every query site which dimension it reads; claim rows additionally carry an empty file list, so even a forgotten filter cannot produce a path conflict.
- **An unclaimed workspace gates nothing.** The claim arbitrates *between* operators who want one; it is not a mandatory ceremony before every dispatch. Requiring one unconditionally would break every existing unattended dispatch and would make "the refusal names the current holder" meaningless. Refusal happens only when an active claim exists and the caller presents no token or a stale one.
- **`orbit run ship-sweep` stands down rather than failing** when it meets a held claim: the unattended sweep carries no token and does not force, so a claimed workspace is reported as skipped with the holder, not as a sweep error.

Surfaces: `claim_token` on the `orbit.workflow.ship` / `orbit.workflow.run.resume` tools, `orbit run ship --claim-token`, the dashboard ship and resume bodies, and `ORBIT_WORKSPACE_CLAIM_TOKEN` for an operator shell. Acquire / release / force-release / status are the `orbit.workspace.claim.*` tools, registered inactive alongside `orbit.task.locks.*` — operator-reachable through `orbit tool run`, absent from the agent MCP surface. `orbit.workspace.claim.release` is a governed operation requiring `Operator`, because force-release displaces whoever is driving dispatch.

### Consequences

- Concurrent dispatch by independent orchestrators becomes impossible rather than merely discouraged, and the unguarded discovery-mode submission path is covered by construction rather than by a second bespoke check.
- Multi-operator use of one workspace becomes coherent: filing and inspecting work stays open, so several people can work different features while exactly one drives execution.
- **Cost:** a third exclusivity concept joins declared ownership and run leases. Orbit will hold reservations, leases, and claims — all TTL-bounded exclusive holds at different granularities. Without deliberate vocabulary discipline these will be confused for one another.
- **Cost:** a dead holder blocks dispatch until the TTL elapses. Force-release is the necessary escape hatch and also the thing that weakens the guarantee, since a habitual force-release makes the claim advisory in practice.
- **Cost:** the claim token is state the holder must keep. A client that loses it must wait out the TTL or force, even though it is the legitimate holder.
- **Cost:** two coordination dimensions now share one table. The discriminator is compiler-enforced at every query site and claim rows carry no files, but a future reader of `task_reservations` must still learn that the table holds two things.
- **Cost:** this contradicts the resident-orchestrator design, which chose one-active-epic plus non-overlapping routine fires plus a host pin specifically to avoid introducing a lease or assignee subsystem. That decision is revised, not left standing in contradiction.
- Declared ownership stays what it is. The claim answers "which operator is driving this workspace right now", not "which machine holds the canonical checkout", and the two must not be collapsed.

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

**Status:** Accepted · 2026-08 · [ORB-10730] implemented the dormant fleet boundary and local routine-pin validation; supersedes [ADR-0230]; defers parts of [ADR-0227], [ADR-0228]; amends [ADR-0231]
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

## ADR-0243 — Drop the autonomous spoke runner track in favor of orchestrator-driven dispatch

**Status:** Proposed · 2026-07-19 21:59:19.974550Z · [ORB-10281], [ORB-10282], [ORB-10283], [ORB-10284], [ORB-10246], [ORB-10269]
**Owner:** human
**Created:** 2026-07-19 21:59:19.974550Z
**Last updated:** 2026-07-19 21:59:19.974550Z
**Tags:** `host-registry`, `mcp-bridge`, `runner`, `execution-placement`

### Context

The I/J wave of the ORB-10246 multi-host plan (ORB-10281 hub run leases, ORB-10282 one-shot spoke runner journal and leased executor, ORB-10283 durable report recovery and fault injection, ORB-10284 minute-clock integration) was designed for unattended pull-based execution: a spoke machine polls the hub, leases runs, survives crashes via a durable local journal, and reports back with strict ack-before-spawn ordering — all with nobody watching.

Since 2026-07-12 the operating model is deliberately the opposite: dispatch is orchestrator-driven with no scheduled ship-sweep. The orchestrator triages, dispatches explicit task ids via workflow_ship, and shepherds every run to done (ship-shepherd / run-rescue). The orchestration layer is already the durability and recovery authority; a spoke-side lease journal would duplicate that one layer down. Additionally, all execution today happens on the machine hosting the hub (worker via bridge, or direct SSH sessions), so no second machine needs to lease work. The lease/journal/poll machinery (I2/I3) is among the hardest, most stateful code in the plan, and none of the four tasks has started.

### Decision

Cancel the runner-lease track: ORB-10281, ORB-10282, ORB-10283, and ORB-10284. Do not build autonomous spoke polling, run leases, runner-only MCP tools, the spoke runner journal, or runner/routine clock composition.

If a second execution machine materializes, reach it with supervised push-style invocation (agent_invoke over the existing SSH-carried MCP link from ORB-10269, which lands independently and is retained), accepting that a crashed remote run is re-dispatched by the shepherd rather than resumed from a local journal.

### Consequences

- The multi-host plan under ORB-10246 shrinks to the landed registry/broker/knowledge waves; remaining H/G units should be re-audited for dependencies they assumed on I/J (placement-aware submission ORB-10280 in particular) before promotion.
- Unattended crash-resumable execution on remote spokes is forgone; recovery for any future remote execution is re-dispatch by the orchestrator, which can duplicate side effects of a partially completed run — acceptable because runs land through PR-gated review or are otherwise idempotent at the task level.
- Cost: if a genuinely unattended multi-machine fleet is ever needed, this track must be re-scoped and re-planned; the cancelled task specs remain in the store as the starting point, but design context will have aged.
- Roll-forward: this decision reverses cheaply — no code was built, no schema shipped, and ORB-10269's transport remains available for either push or pull designs.

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
- [ORB-10730] — withdrew fleet host administration and workspace linking from
  the v1 command graph, removed registry/cache reads from v1 discovery and
  routines, and retained the underlying aliases, retirement, projections,
  cache codecs, and tables as explicitly documented dormant v2 modules.
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
