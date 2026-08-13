---
title: Orbit MCP Bridge — Overview
owner: claude
last_updated: 2026-08-13
last_validated: 2026-08-02
status: Draft
feature: mcp-bridge
doc_role: overview
type: design
summary: One canonical local Orbit MCP front door that routes coordination to the machine that owns the workspace, keeps owner-authored knowledge and derived indexes local, reaches checkoutless clients over an owned SSH tunnel, and removes Bridge's duplicated Orbit parity layer.
tags: [mcp, remote-access, host-registry, bridge, multi-host]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**", "crates/orbit-common/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access, orbit-search]
related_artifacts: [ORB-00424, ORB-10262, ORB-10268, ORB-10319, ORB-10690, ADR-0181, ADR-0199, ADR-0200, ADR-0201, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0348, ADR-0350, ADR-0351, ADR-0355, ADR-0356, ADR-0357, ADR-0358]
---

# Orbit MCP Bridge — Overview

> **Learning-subsystem retirement.** [ORB-10736] / [ADR-0359] remove the native
> project-learning resource. Every learning-specific route, sidecar, placement,
> and payload described below is retained only as retired historical context and
> is not part of the current MCP contract.

> **Status: Draft — structural rewrite in flight.** The singular-hub contract
> ([ADR-0226], [ADR-0229], [ADR-0230]) is superseded by [ADR-0355]–[ADR-0358],
> recorded in [../host-registry/4_decisions.md](../host-registry/4_decisions.md).
> Every machine is now its own coordination host for the workspaces it owns.
> Sections describing execution placement, run leases, the presence map, and host
> registration are **deferred to v2** — retained as history, not as design.

Orbit should expose one canonical MCP contract through a local broker on every
machine, with at most one cross-machine destination: the machine that owns the
workspace. Tasks are created and read there; friction, learning, and
workspace-registry state are local to the owning machine. Knowledge records are
authored and read current on the workspace owner; code graph and docs-derived
operations stay on the checkout where the agent is running. No machine forwards a
call on another's behalf; a client reaches an owner machine directly or not at
all.

When the workspace owner is another machine, a client may point at that machine's
MCP over the existing SSH route for task creation and reads. It does not translate
through the dashboard HTTP API, synchronize stores, or discover per-workspace
network routes. Bridge stops redeclaring Orbit tools and remains only for
constellation capabilities Orbit does not own.

That shape describes machines with a checkout to protect. A checkoutless client,
such as an off-box orchestrator, holds no local-derived state and does not
participate in owner routing. It reaches Orbit through an owned SSH tunnel
terminating at a loopback-bound listener, where calls resolve remotely with no
placement routing ([ADR-0350]). The tunnel adds one operation — running a command
— requiring both operator capability and the workspace claim ([ADR-0351],
[host-registry/2_design.md §3.2](../host-registry/2_design.md)). The advertised
per-tool surface is unchanged. See [2_design.md §5.3](./2_design.md).

This feature and [host-registry](../host-registry/1_overview.md) are coupled.
Host-registry declares machine identity, the machine-scoped task-id prefix, and
per-workspace ownership in the machine-local workspace registry. MCP bridge turns
those declarations into one local tool surface.

Their implementation is coupled as well: `orbit-remote` owns workspace-registry
persistence, canonical schema composition, the broker, the owner-machine MCP
route, and learning integration. `orbit-mcp` is the neutral RMCP framing/raw-client
kernel; `orbit-store` is the neutral SQLite/feature-migration kernel; `orbit-core`
retains transport-independent execution. This vertical boundary replaces the
earlier registry-only extraction without creating either a separate broker crate
or a second database ([ORB-10319], [ADR-0240]).

## 1. Motivation

Bridge currently presents an Orbit-shaped MCP surface by vendoring Orbit schemas,
redeclaring arguments in Python, calling Orbit's dashboard HTTP API, and reshaping
responses. That gave off-box clients useful coverage quickly, but it created two
contracts and two execution paths. Missing HTTP fields, provenance gaps, incomplete
knowledge/search behavior, and snapshot refreshes are consequences of that split,
not isolated endpoint bugs ([ORB-00424]).

A direct `ssh dk1 orbit mcp serve` replacement is also incomplete: it moves graph
and docs queries away from the branch/worktree the agent is using, and it implies
one machine can serve current knowledge for every workspace. The host-registry
design explicitly says otherwise:

1. **Coordination is owner-local.** Each machine coordinates the workspaces it
   owns, and refuses coordination writes for the ones it does not.
2. **Knowledge is owner-authored and workspace-scoped.** The owner allocates the
   artifact key within the workspace and writes locally. A machine serves current
   knowledge only for workspaces it owns; it never proxies to other owners.
3. **Derived state is checkout-local.** Graph, docs index, semantic companions, and
   routine scheduler state describe a particular local checkout.
4. **Execution placement is out of scope in v1** (deferred to v2 with run leases
   and the `runner` capability, [ADR-0358]). MCP bridge opens no machine-to-machine
   command channel.

The required shape is one Orbit-owned contract with explicit placement and at most
one network edge per client: client → owner machine.

## 2. Core Concepts

- **Local MCP broker** — the `orbit mcp serve` process registered with the client.
  It preserves the exact local checkout, reads the machine-local workspace registry
  to determine ownership, filters by capability, and dispatches each tool by
  placement class.
- **Owner route** — execution on the machine that owns the workspace, in-process
  when that is this machine. Otherwise coordination writes fail closed and name the
  owner; in v1 only task creation and reads may be sent to a remote owner over the
  SSH route.
- **Local-derived route** — execution against rebuildable state derived from the
  current checkout. Graph and docs never cross the owner route.
- **Composite route** — a tool that deliberately performs more than one placement.
  `orbit.search` is the only v1 composite; knowledge creation stopped being one
  when the allocation step disappeared ([ADR-0357]).
- **Placement class** — canonical metadata on each Orbit tool definition: `owner`,
  `local-derived`, or `composite`. Placement is independent of capability.
- **Owner link** — an SSH-carried MCP connection to an owner machine's stable
  `machine_id`, used in v1 only for task creation and reads.
- **Checkoutless client** — an MCP client with no local checkout: it owns no
  workspace and holds no local-derived state. It does not participate in owner
  routing.
- **Owned tunnel** — an SSH tunnel Orbit establishes or reuses to a loopback-bound
  listener on a remote machine. It is reusable infrastructure rather than one
  consumer's implementation detail, and it carries no placement routing.
- **Explicit replica read** — an opt-in read from a pulled/reindexed Git knowledge
  replica. It is never presented as current and never selected automatically.
- **Caller-host provenance** — originating host identity propagated to the owner
  machine's audit, separately from the serving process host.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Generic MCP wire adapter and raw stream client | [crates/orbit-mcp/](../../../crates/orbit-mcp/) | [ORB-00424] |
| Local broker, trusted owner-route config/link pool, graph and learning composition, safe surface, and audit boundary | [crates/orbit-remote/src/mcp/](../../../crates/orbit-remote/src/mcp/) | [ORB-10262], [ORB-10268], [ORB-10269] |
| Generic builtin schema + placement metadata | [crates/orbit-tools/src/builtin/orbit/mod.rs](../../../crates/orbit-tools/src/builtin/orbit/mod.rs) | [ORB-00424] |
| Canonical workspace discovery schema, placement, and projection | [crates/orbit-remote/src/mcp/discovery.rs](../../../crates/orbit-remote/src/mcp/discovery.rs) | [ORB-10267] |
| Local owner/builtin execution + transport-independent coordination executor | [crates/orbit-core/src/](../../../crates/orbit-core/src/) | [ORB-00424], [ORB-10319] |
| Registry SQL and remote audit/snapshot persistence over shared `orbit.db` | [crates/orbit-remote/src/persistence/](../../../crates/orbit-remote/src/persistence/) | [ORB-10319] |
| Machine identity, task-id prefix, and workspace ownership | [host-registry/2_design.md](../host-registry/2_design.md) | [ORB-00424] |
| Session workspace and caller-host metadata | [mcp-session-context/2_design.md](../mcp-session-context/2_design.md) | [ORB-00424] |
| Existing SSH-over-loopback posture | [remote-access/2_design.md](../remote-access/2_design.md) | — |
| Cross-kind search merge behavior | [orbit-search/2_design.md](../orbit-search/2_design.md) | — |

Detailed topology, knowledge semantics, routing, configuration, and migration are in
[2_design.md](./2_design.md). Open directions are in [3_vision.md](./3_vision.md).

## Task References

- [ORB-00424] — proposed replacing Bridge's duplicated Orbit parity layer with a
  canonical local/remote Orbit MCP surface. The hub/owner split and star topology
  it was revised around are superseded by the v1 ownership model; the canonical
  contract and placement metadata survive.
- [ORB-10268] — implemented the strict machine-global hub trust document and the
  fixed, checkoutless, non-recursive hub MCP server boundary (hub-mode endpoint
  superseded by the v1 owner-machine model; the trust document survives as the
  client's per-route policy).
- [ORB-10269] — implemented contract-pinned SSH MCP links, bounded per-capability
  reuse, exact caller/workspace correlation, and no-replay outcome handling. The
  transport survives; its single fixed hub target does not.
- [ORB-10319] — consolidates registry persistence and the MCP bridge implementation
  in vertical `orbit-remote`, leaving MCP, Store, Core, Tools, and Common as neutral
  acyclic dependencies ([ADR-0240]). Unaffected by this revision.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
