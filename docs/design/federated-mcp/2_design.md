---
title: Federated MCP — Design
owner: grok
last_updated: 2026-08-23
last_validated: 2026-08-23
status: Draft
feature: federated-mcp
doc_role: design
type: design
summary: Proposed (not shipped) federated MCP mux, selector, capability split, list schema, and fail-closed routing. V1 current behavior stays in mcp-bridge.
tags: [federated-mcp, mcp, host-registry, multi-host]
paths: ["crates/orbit-mcp/**", "crates/orbit-registry/**", "crates/orbit-core/**"]
related_features: [federated-mcp, host-registry, mcp-bridge, remote-access, mcp-session-context]
related_artifacts: [ORB-11009, ORB-11008]
---

# Federated MCP — Design

This document describes the **proposed** federated MCP surface. It is not implemented. Current v1 behavior — one chosen destination, byte-transparent direct SSH stdio, no Orbit process relaying onward — remains [mcp-bridge 2_design.md](../mcp-bridge/2_design.md). Do not read the sections below as live runtime.

The prescriptive invariants live in [specs/federated-workspace-mcp.md](./specs/federated-workspace-mcp.md). This file explains how those pieces fit.

## 1. Operator-configured destinations

The gateway is a mux in front of destinations the operator already configured (MCP remotes or SSH stdio targets). It does not:

- grow host-registry into a fleet inventory;
- auto-discover the owner checkout of a repository;
- place work, elect a leader, or pick a healthy substitute.

Direct SSH stdio to one chosen host remains the v1 remote path and is unchanged by this proposal. A caller that does not use the federated namespace never hits the mux.

## 2. Host-qualified selector

Every workspace-scoped federated call carries an opaque host-qualified selector. The token is addressing data. Callers treat it as opaque. The gateway must not reinterpret it against its own local catalog.

The stable key is `machine_id` (`hm_…`), not renameable `host_id`. Example form: `hm_<id>/ws_orbit`. A display host name such as `orbit-linux/ws_orbit` is not a valid selector.

The destination is ambiguous when two configured destinations share a `machine_id`, or when the token is not uniquely host-qualified (a bare `ws_*` is not enough). Ambiguous tokens fail; they are not disambiguated by local catalog, cwd, or session default.

## 3. Capabilities mapped onto owner and replica

This feature does not invent a second ownership model. Host-registry already distinguishes:

- **owner checkout** — local checkout whose logical `owner_machine_id` equals this machine;
- **replica checkout** — local checkout that names another machine as the logical owner.

Those roles map onto capability classes:

| Catalog role | Capability class | Meaning |
|---|---|---|
| Owner checkout | `control_plane` (and typically also `execute`) | Control-plane authority for that workspace's coordination; the canonical checkout can also run |
| Replica checkout | `execute` | Execution binding only |

A destination Core that does not hold a class **refuses** tools of that class with a named error (`capability_refused`). The gateway may advertise the destination's capabilities; it does not enforce by rewriting the destination or failing over to the owner.

If a caller routes `orbit_task_add` to an execution-binding host, that destination refuses. The gateway must not silently send the call to the owner.

## 4. Split authority — "mutations" is not one blob

Routing changes where a call is delivered; it does not move authority.

- **Destination host** is authoritative for **runs, logs, and scheduler state** on that host.
- **Declared control-plane authority** is authoritative for **task issuance and the coordination store**. That store may later be cloud-offloaded; that is an open question, not a hidden replica protocol.

Do not lump those as one "mutations" blob. A replica can accept execute-class work against its own run/log/scheduler state and must still refuse control-plane task issuance.

`orbit_workspace_list` (federated) is an aggregate of live descriptors, not an aggregate task or store query.

## 5. Federated workspace list

Federated `orbit_workspace_list` is **session-unbound**: it must not require a workspace selector. The result is **additive** on today's v1 list fields (`id`, `name`, `ship_mode`, `owner_machine_id`, `git_remote`, `base_branch`, `status`, timestamps) plus:

- `selector` — opaque host-qualified route token;
- `host` — destination display identity (`host_id`, renameable, display only);
- `machine_id` — destination stable identity;
- host-reachability — SSH/MCP reachability of the configured destination;
- workspace checkout-health — repo-root presence at that destination, the same narrow rule as host-registry;
- `capabilities` — classes the destination currently advertises for that workspace.

Do not overload one `health` field with both SSH reachability and repo-root presence. A down or unreachable host is **included** with an explicit unreachable/unhealthy projection, not omitted. Omission makes every later call a stale-route surprise.

## 6. Fail-closed routing

The gateway delivers a workspace-scoped call to the destination encoded in the selector. It fails explicitly for:

- unknown selector;
- unreachable destination;
- unhealthy checkout;
- ambiguous destination;
- stale route (destination no longer advertises that workspace).

It must not fall back to a local workspace, another host with a matching `ws_*`, a default workspace, or a cached host-local runtime.

`tool_not_on_this_host` is a distinct error from `unknown_selector`: the destination is identified, but the tool is not advertised there.

## 7. Relationship to v1 MCP

v1 local stdio, direct SSH stdio, and `orbit mcp listen` stay as specified in mcp-bridge. The mux is an explicit exception to v1 "byte-transparent / no Orbit process relays onward", **for the federated namespace only**. Automatic owner discovery, replication, relays-as-product, and fleet placement stay out. See [mcp-bridge 3_vision.md §5](../mcp-bridge/3_vision.md).

## 8. Concerns & Honest Limitations

- This design is not shipped. No runtime, tool schema, or conformance YAML in this change implements it.
- Availability is bounded by the chosen destination. Callers must handle visible routing failures; there is no transparent substitute result.
- Including unreachable hosts makes the list honest and larger; clients must read reachability rather than treating presence as liveness.
- Capability advertisement can lag destination Core. The destination refuse is the correctness boundary; the gateway is not a second authorization layer.
- Transport authentication, selector expiry, health freshness, and cloud coordination-store details are deliberately unresolved. See [3_vision.md](./3_vision.md).
- Mixed Orbit versions across destinations can advertise different surfaces; `tool_not_on_this_host` and `capability_refused` must remain distinguishable from "the mux is confused."

## Task References

- [ORB-11008] — recorded the federated multi-host MCP policy
- [ORB-11009] — specified this proposed mechanism as the implementable contract

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
