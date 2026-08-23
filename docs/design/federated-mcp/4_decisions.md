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
related_artifacts: [ORB-11010, ORB-11009, ORB-11008]
---

# Federated MCP — Decisions

Record non-obvious decisions here by title. These are Door 2 standing rules. Code anchors: `crates/orbit-mcp/src/federated/` (`FederatedMcpHost`, destinations file, host-qualified selector, live probe, fail-closed routing). See [CONVENTIONS.md §4](../CONVENTIONS.md#4-decisions).

## Federated MCP is a mux of operator-configured destinations

**Recorded:** 2026-08 · [ORB-11009] · [ORB-11010] (PR #1139)

### Context

A single MCP namespace can look like a fleet catalog. Host-registry already owns machine-local identity and checkout roles. Growing that catalog into routing, or auto-discovering owners, would make the gateway a second inventory and a placement service.

### Decision

Treat the federated surface as a mux in front of destinations the operator already configured. It is not a host-registry evolution, not a new fleet inventory, and not automatic owner discovery. Direct SSH stdio to one chosen host remains v1. Apply this whenever a future change is tempted to register, probe, or place hosts inside the gateway.

### Consequences

- Destination membership is an operator configuration problem, not a catalog schema problem.
- Cost: the mux cannot "just find" an owner or a healthy replica; a missing destination is a configuration gap, not a discovery miss.

## Host-qualified selectors are structured and caller-uninterpreted

**Recorded:** 2026-08 · [ORB-11009] · [ORB-11010] (PR #1139)

### Context

`host_id` is renameable display. Keying a route on it would invalidate every selector on `orbit host rename` and invite examples such as `orbit-linux/ws_orbit`. Treating the token as a formless blob hid that the encoding `hm_<id>/ws_*` is normative. The gateway's local catalog is the wrong resolution authority for another machine's workspace.

### Decision

Key the host-qualified selector on stable `machine_id` (`hm_…`). Encoding `hm_<id>/ws_*` is normative. Callers treat the token as **structured, caller-uninterpreted**: they must not parse it, must not construct it from `host_id`, and must not concatenate remembered `machine_id` and `id` values. The only caller-facing way to obtain a selector is to copy the `selector` field from federated `orbit_workspace_list`. The gateway must not reinterpret it against its own local catalog. A token that is not uniquely host-qualified (a bare `ws_*`) is `unknown_selector`. Duplicate `machine_id` across destinations is config-load `ambiguous_destination`, not a per-call outcome. Apply this to every new selector encoding, including future transport wrappers.

### Consequences

- Renames do not rebind routes; callers can persist selectors across display-name changes.
- Cost: humans cannot mint a selector from a hostname they remember, and they cannot assemble one from listed `machine_id` + `id` either; they must copy the list `selector` field.

## Capability class is assigned by tool behavior, held by catalog role

**Recorded:** 2026-08 · [ORB-11009] · [ORB-11010] (PR #1139)

### Context

One namespace plus several checkouts of one repository looks like a synchronized task store unless capability and authority are named separately. Lumping "mutations" would send `orbit_task_add` to a replica, or silently fail over to the owner, and invent a second ownership model. Classifying only `orbit_task_add` would leave every other tool for an implementer to guess. Treating list advertisement as the source of truth is circular because advertisement can lag Core, and `owner_machine_id` is `Option`.

### Decision

Assign capability class by what the tool does: task issuance and coordination-store writes are `control_plane`; tools that touch runs, logs, or scheduler state are `execute`; discovery and list tools are unclassified and are not subject to `capability_refused`. Do not add a per-tool registry field for this. The destination's **local catalog role** determines which classes that destination holds; list advertisement is a hint that may lag. Destination Core refusal is the correctness boundary. Owner checkout holds `control_plane` and may also hold `execute` when it runs locally — that second class is not a refusal input. Replica checkout holds `execute`. A workspace with absent `owner_machine_id` cannot advertise `control_plane`. Destination-host Core refuses the other class with `capability_refused` and no implicit failover. Split remaining authority: the destination host owns runs, logs, and scheduler state; the declared control-plane owns task issuance and the coordination store. Apply this to every new federated tool, not only `orbit_task_add`.

### Consequences

- Owner/replica remain the only ownership vocabulary; execute-class work can stay on a replica without cloning the coordination store.
- Cost: a caller that picks the wrong selector gets a named refusal instead of a successful write on the "right" host; clients must route control-plane tools to a `control_plane` destination themselves, and they cannot trust a stale list advertisement over the destination refuse.

## Unreachable destinations stay in the list and routing fails closed

**Recorded:** 2026-08 · [ORB-11009] · [ORB-11010] (PR #1139)

### Context

Omitting a down host from `orbit_workspace_list` makes every later call a stale-route surprise. Calling the federated list "additive" hid that v1 puts `machine_id` on the envelope and filters to Active-and-locally-checked-out workspaces. Overloading one `health` field hides whether SSH failed or the repo root is gone. Falling back to another host with the same `ws_*` is a replica protocol by another name. Overlapping error classes without precedence would let an implementation pick whichever name was convenient.

### Decision

Federated `orbit_workspace_list` is a new session-unbound shape, not a compatible extension of v1: `machine_id` lives on each descriptor, not the envelope, and the v1 Active-and-locally-checked-out filter is not inherited. Configured workspaces on unreachable or inactive destinations are included. Host-reachability and checkout-health are separate fields. Routing decides on live delivery, not cached list health. Caller-facing precedence is `unknown_selector` → `ambiguous_destination` (config) → `unreachable_destination` → `stale_route` → `unhealthy_checkout` → `tool_not_on_this_host` → `capability_refused`. Unreachable wins over capability and stale because those are undecidable without the host. `tool_not_on_this_host` is distinct from `unknown_selector`. No local fallback, no default workspace, no cached host-local runtime. Probe cadence stays a vision open question. Apply this to every new federated discovery field and every new routing miss.

### Consequences

- Callers can distinguish "host down" from "checkout missing" from "tool not advertised here."
- Cost: the list is not a set of live, callable workspaces; clients must read reachability and capabilities before routing, and availability is bounded by the chosen destination.

## Single control-plane per repository is operator configuration

**Recorded:** 2026-08 · [ORB-11010] (PR #1139)

### Context

Owner role is machine-local. Two hosts can each declare Owner. Independently inited checkouts have different `ws_*`, so the mux cannot observe the collision without the fleet discovery this design forbids. Listing "competing authorities" as a mux-enforced non-goal would make an implementer invent detection the architecture cannot perform.

### Decision

A single control-plane per repository is an operator configuration responsibility, not a mux invariant. The mux does not detect competing owners and does not raise an error when they exist. The would-be signal is matching `git_remote` across destinations with differing `owner_machine_id`. A violation surfaces as two independent control planes, not an error. Apply this whenever a future change is tempted to compare `git_remote` values inside the gateway or to refuse a second Owner.

### Consequences

- Operators who configure one owner per repository get one control plane; that is a deployment rule, not software.
- Cost: a misconfigured pair of destinations will accept conflicting task issuance with no mux warning.

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
- [ORB-11009] — recorded these standing rules as the contract home (PR #1139)
- [ORB-11010] — closed the PR #1139 review holes (selector wording, tool class, error precedence, competing authorities)

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
