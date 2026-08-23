---
title: Federated MCP — Decisions
owner: grok
last_updated: 2026-08-23
last_validated: 2026-08-23
status: Draft
feature: federated-mcp
doc_role: decisions
type: design
summary: Standing rules for the proposed federated MCP mux: destinations are configured, selectors use machine_id, authority is split, routing fails closed.
tags: [federated-mcp, mcp, host-registry, multi-host]
paths: ["crates/orbit-mcp/**", "crates/orbit-registry/**", "crates/orbit-core/**"]
related_features: [federated-mcp, host-registry, mcp-bridge, remote-access]
related_artifacts: [ORB-11009, ORB-11008]
---

# Federated MCP — Decisions

Record non-obvious decisions here by title. These are Door 2 standing rules for future implementation; no code anchors yet because the surface is not shipped. See [CONVENTIONS.md §4](../CONVENTIONS.md#4-decisions).

## Federated MCP is a mux of operator-configured destinations

**Recorded:** 2026-08 · [ORB-11009]

### Context

A single MCP namespace can look like a fleet catalog. Host-registry already owns machine-local identity and checkout roles. Growing that catalog into routing, or auto-discovering owners, would make the gateway a second inventory and a placement service.

### Decision

Treat the federated surface as a mux in front of destinations the operator already configured. It is not a host-registry evolution, not a new fleet inventory, and not automatic owner discovery. Direct SSH stdio to one chosen host remains v1. Apply this whenever a future change is tempted to register, probe, or place hosts inside the gateway.

### Consequences

- Destination membership is an operator configuration problem, not a catalog schema problem.
- Cost: the mux cannot "just find" an owner or a healthy replica; a missing destination is a configuration gap, not a discovery miss.

## Host-qualified selectors are keyed by machine_id

**Recorded:** 2026-08 · [ORB-11009]

### Context

`host_id` is renameable display. Keying a route on it would invalidate every selector on `orbit host rename` and invite examples such as `orbit-linux/ws_orbit`. The gateway's local catalog is the wrong resolution authority for another machine's workspace.

### Decision

Key the opaque host-qualified selector on stable `machine_id` (`hm_…`), for example `hm_<id>/ws_orbit`. Callers treat it as opaque. The gateway must not reinterpret it against its own local catalog. Two configured destinations that share a `machine_id`, or a token that is not uniquely host-qualified, are `ambiguous_destination`. Apply this to every new selector encoding, including future transport wrappers.

### Consequences

- Renames do not rebind routes; callers can persist selectors across display-name changes.
- Cost: humans cannot mint a selector from a hostname they remember; they must copy `machine_id` from the list (or a configured destination record).

## Capability class and mutation authority are split

**Recorded:** 2026-08 · [ORB-11009]

### Context

One namespace plus several checkouts of one repository looks like a synchronized task store unless capability and authority are named separately. Lumping "mutations" would send `orbit_task_add` to a replica, or silently fail over to the owner, and invent a second ownership model.

### Decision

Map capabilities onto existing catalog roles: owner checkout is `control_plane` (and typically `execute`); replica checkout is `execute`. Destination-host Core refuses the other class with `capability_refused` and no implicit failover. Split remaining authority: the destination host owns runs, logs, and scheduler state; the declared control-plane owns task issuance and the coordination store. The gateway may advertise capabilities; it does not rewrite destinations. Apply this to every new federated tool, not only `orbit_task_add`.

### Consequences

- Owner/replica remain the only ownership vocabulary; execute-class work can stay on a replica without cloning the coordination store.
- Cost: a caller that picks the wrong selector gets a named refusal instead of a successful write on the "right" host; clients must route control-plane tools to a `control_plane` destination themselves.

## Unreachable destinations stay in the list and routing fails closed

**Recorded:** 2026-08 · [ORB-11009]

### Context

Omitting a down host from `orbit_workspace_list` makes every later call a stale-route surprise. Overloading one `health` field hides whether SSH failed or the repo root is gone. Falling back to another host with the same `ws_*` is a replica protocol by another name.

### Decision

Federated `orbit_workspace_list` is session-unbound and additive. Host-reachability and checkout-health are separate fields. A down or unreachable host is included with an explicit projection, not omitted. Unknown, unreachable, unhealthy, ambiguous, and stale routes fail explicitly. `tool_not_on_this_host` is distinct from `unknown_selector`. No local fallback, no default workspace, no cached host-local runtime. Apply this to every new federated discovery field and every new routing miss.

### Consequences

- Callers can distinguish "host down" from "checkout missing" from "tool not advertised here."
- Cost: the list is not a set of live, callable workspaces; clients must read reachability and capabilities before routing, and availability is bounded by the chosen destination.

## The mux is a federated-namespace exception to v1 no-relay

**Recorded:** 2026-08 · [ORB-11009]

### Context

mcp-bridge v1 forbids an Orbit process relaying a call onward and requires a byte-transparent SSH proxy. A mux necessarily inspects the selector and forwards. Treating that as a general relay product, or leaving vision §5 as "multi-host routing is a separate product," would either block the surface or un-forbid owner discovery, replication, and fleet placement.

### Decision

Admit the mux as an explicit exception to v1 byte-transparent / no-relay rules **for the federated namespace only**. Automatic owner discovery, replication, relays-as-product, and fleet placement stay out. v1 current-behavior docs continue to describe v1. Apply this whenever a later change wants the proxy to inspect, filter, or redirect traffic outside the federated namespace.

### Consequences

- Implementation can build the mux without rewriting mcp-bridge 2_design as if v1 already federated.
- Cost: two MCP entry shapes must be documented and tested — v1 direct SSH stays policy-free; only the federated namespace may route — and mixed use cannot leak mux policy into the byte-transparent proxy.

## Task References

- [ORB-11008] — recorded the prior federated MCP policy that these rules implement
- [ORB-11009] — recorded these standing rules as the contract home

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
