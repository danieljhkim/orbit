---
title: Orbit MCP Bridge — Overview
owner: codex
last_updated: 2026-08-09
last_validated: 2026-08-02
status: Accepted
feature: mcp-bridge
doc_role: overview
type: design
summary: One canonical local Orbit MCP front door that routes coordination to a single hub, keeps owner-authored knowledge and derived indexes local, reaches checkoutless clients over an owned SSH tunnel, and removes Bridge's duplicated Orbit parity layer.
tags: [mcp, remote-access, host-registry, bridge, multi-host]
paths: ["crates/orbit-remote/**", "crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-tools/**", "crates/orbit-store/**", "crates/orbit-common/**"]
related_features: [mcp-bridge, host-registry, mcp-session-context, remote-access, orbit-search]
related_artifacts: [ORB-00424, ORB-10262, ORB-10268, ORB-10319, ORB-10690, ADR-0181, ADR-0199, ADR-0200, ADR-0201, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240, ADR-0348, ADR-0350, ADR-0351]
---

# Orbit MCP Bridge — Overview

Orbit should expose one canonical MCP contract through a local broker on every
machine, with exactly one cross-machine destination: the main host (hub). Tasks,
frictions, registry operations, workflow dispatch, run state, and global ID
allocation go to the hub. Knowledge records are authored and read current on the
workspace owner; code graph and docs-derived operations stay on the checkout where
the agent is running. The hub never forwards a call to an owner, and no spoke talks
to another spoke.

When the hub is remote, the local broker carries hub-class calls over SSH to a
hub-mode Orbit MCP process. It does not translate through the dashboard HTTP API,
synchronize stores, or discover per-workspace network routes. Bridge stops
redeclaring Orbit tools and remains only for constellation capabilities Orbit does
not own.

That topology describes **spokes** — machines with a checkout to protect. A
checkoutless client, such as an off-box orchestrator, holds no local-derived state
and is not a spoke. It reaches Orbit through an owned SSH tunnel terminating at a
loopback-bound listener, where calls resolve remotely with no placement routing
([ADR-0350]). The tunnel carries three operations — enumerate the registry, invoke
a tool by name, and run a command — the last requiring both operator capability
and the workspace claim ([ADR-0351],
[host-registry/2_design.md §3.2](../host-registry/2_design.md)). The existing
advertised per-tool surface is retained pending measurement. See
[2_design.md §5.3](./2_design.md).

This feature and [host-registry](../host-registry/1_overview.md) are coupled.
Host-registry declares the hub, workspace owner, local replica role, execution
placement, and pull-based run lease. MCP bridge turns those declarations into one
local tool surface while preserving the star topology.

Their implementation is coupled as well: `orbit-remote` owns registry persistence,
canonical schema composition, the broker, trusted hub server/link, graph and
learning integration, and spoke registration. `orbit-mcp` is the neutral RMCP
framing/raw-client kernel; `orbit-store` is the neutral SQLite/feature-migration
kernel; `orbit-core` retains transport-independent execution. This vertical boundary
replaces the earlier registry-only extraction without creating either a separate
broker crate or a second database ([ORB-10319], [ADR-0240]).

## 1. Motivation

Bridge currently presents an Orbit-shaped MCP surface by vendoring Orbit schemas,
redeclaring arguments in Python, calling Orbit's dashboard HTTP API, and reshaping
responses. That gave off-box clients useful coverage quickly, but it created two
contracts and two execution paths. Missing HTTP fields, provenance gaps, incomplete
knowledge/search behavior, and snapshot refreshes are consequences of that split,
not isolated endpoint bugs ([ORB-00424]).

A direct `ssh dk1 orbit mcp serve` replacement is also incomplete: it moves graph
and docs queries away from the branch/worktree the agent is using, and it implies
the hub can serve current knowledge for every workspace. The host-registry design
explicitly says otherwise:

1. **Coordination is hub-only.** Every machine sends task/friction/workflow traffic
   to one hub.
2. **Knowledge is owner-authored.** The owner obtains a global ID from the hub and
   finalizes the record locally. The hub serves current knowledge only for
   workspaces it owns; it never proxies to other owners.
3. **Derived state is checkout-local.** Graph, docs index, semantic companions, and
   routine scheduler state describe a particular local checkout.
4. **Execution placement is pull-based.** The hub queues; the selected machine
   polls and leases. MCP bridge never creates a hub-to-spoke command channel.

The required shape is one Orbit-owned contract with explicit placement and one
network edge per spoke: spoke → hub.

## 2. Core Concepts

- **Local MCP broker** — the `orbit mcp serve` process registered with the client.
  It preserves the exact local checkout, reads the local hub/owner role, filters by
  capability, and dispatches each tool by placement class.
- **Hub route** — execution on the single coordination hub, short-circuited
  in-process when the current machine is the hub and carried over SSH otherwise.
- **Owner route** — execution in the workspace owner's checkout. It is reachable
  only when the owner is this machine or the hub; the broker never opens a route to
  another spoke owner.
- **Local-derived route** — execution against rebuildable state derived from the
  current checkout. Graph and docs never cross the hub link.
- **Composite route** — a tool that deliberately performs more than one placement.
  Knowledge creation (hub ID + owner finalize) and `orbit.search` are the v1
  examples.
- **Placement class** — canonical metadata on each Orbit tool definition: `hub`,
  `owner`, `local-derived`, or `composite`. Placement is independent of
  capability.
- **Hub link** — the one trusted SSH-carried MCP connection from a spoke to the
  stable hub `machine_id`.
- **Checkoutless client** — an MCP client with no local checkout: it owns no
  workspace, registers no host, and holds no local-derived state. It is not a
  spoke and does not participate in hub or owner routing.
- **Owned tunnel** — an SSH tunnel Orbit establishes or reuses to a loopback-bound
  listener on a remote machine. It is reusable infrastructure rather than one
  consumer's implementation detail, and it is deliberately not a hub link.
- **Explicit replica read** — an opt-in read from a pulled/reindexed Git knowledge
  replica. It is never presented as current and never selected automatically.
- **Caller-host provenance** — originating host identity propagated to hub audit,
  separately from the hub process host.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Generic MCP wire adapter and raw stream client | [crates/orbit-mcp/](../../../crates/orbit-mcp/) | [ORB-00424] |
| Local broker, trusted hub config/server/link pool, graph and learning composition, safe surface, and audit boundary | [crates/orbit-remote/src/mcp/](../../../crates/orbit-remote/src/mcp/) | [ORB-10262], [ORB-10268], [ORB-10269] |
| Generic builtin schema + placement metadata | [crates/orbit-tools/src/builtin/orbit/mod.rs](../../../crates/orbit-tools/src/builtin/orbit/mod.rs) | [ORB-00424] |
| Canonical host/workspace discovery schema, placement, and projection | [crates/orbit-remote/src/mcp/discovery.rs](../../../crates/orbit-remote/src/mcp/discovery.rs) | [ORB-10267] |
| Local owner/builtin execution + transport-independent hub coordination executor | [crates/orbit-core/src/](../../../crates/orbit-core/src/) | [ORB-00424], [ORB-10319] |
| Registry SQL and remote audit/snapshot persistence over shared `orbit.db` | [crates/orbit-remote/src/persistence/](../../../crates/orbit-remote/src/persistence/) | [ORB-10319] |
| Hub, ownership, replica role, and run placement | [host-registry/2_design.md](../host-registry/2_design.md) | [ORB-00424] |
| Session workspace and caller-host metadata | [mcp-session-context/2_design.md](../mcp-session-context/2_design.md) | [ORB-00424] |
| Existing SSH-over-loopback posture | [remote-access/2_design.md](../remote-access/2_design.md) | — |
| Cross-kind search merge behavior | [orbit-search/2_design.md](../orbit-search/2_design.md) | — |

Detailed topology, knowledge semantics, routing, configuration, and migration are in
[2_design.md](./2_design.md). Open directions are in [3_vision.md](./3_vision.md).

## Task References

- [ORB-00424] — proposed replacing Bridge's duplicated Orbit parity layer with a
  canonical local/remote Orbit MCP surface; this design revises it around the
  host-registry hub/owner split and star topology.
- [ORB-10268] — implemented the strict machine-global hub trust document and the
  fixed, checkoutless, non-recursive hub MCP server boundary.
- [ORB-10269] — implemented contract-pinned SSH MCP links, bounded per-capability
  reuse, exact caller/workspace correlation, and no-replay outcome handling.
- [ORB-10319] — consolidates registry persistence and the MCP bridge implementation
  in vertical `orbit-remote`, leaving MCP, Store, Core, Tools, and Common as neutral
  acyclic dependencies ([ADR-0240]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
