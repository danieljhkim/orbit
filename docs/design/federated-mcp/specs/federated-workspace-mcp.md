---
type: design
summary: "Spec: Federated workspace MCP mux, selector, capabilities, list schema, and fail-closed routing"
last_validated: 2026-08-23
title: Spec — Federated workspace MCP
owner: grok
status: Draft
feature: federated-mcp
tags: [federated-mcp, mcp, spec]
related_features: [federated-mcp, host-registry, mcp-bridge]
related_artifacts: [ORB-11009, ORB-11008]
---

# Spec: Federated workspace MCP

The federated MCP surface is a **mux of operator-configured destinations**. It presents one caller-facing MCP namespace, lists every configured destination's workspaces as live descriptors, and routes an opaque host-qualified selector to the encoded destination. It is **not** a host-registry evolution, **not** a fleet inventory, and **not** automatic owner discovery. Direct SSH stdio to one chosen host remains v1. This spec is proposed; it is not implemented.

## Why This Exists

Without this contract, an implementation will key selectors on renameable `host_id`, invent a second ownership model, lump runs and task issuance as one "mutations" blob, omit down hosts from the list, or violate mcp-bridge's no-relay rule (or ship a mux the corpus still forbids). [ORB-11008] recorded the policy; this spec is the implementable mechanism from [ORB-11009].

## Mux, not fleet registry

1. Destinations are operator-configured MCP or SSH remotes. The gateway does not register, retire, or enumerate a fleet of machines as host-registry records.
2. The gateway does not auto-discover the owner checkout of a repository and does not perform placement.
3. The gateway must not reinterpret a selector against its own local catalog.
4. A caller that chooses one host and speaks v1 MCP (local stdio, direct SSH stdio, or `orbit mcp listen`) never enters this mux.

## Selector identity

1. The opaque host-qualified selector is keyed by stable `machine_id` (`hm_…`), not renameable `host_id`.
2. Example form: `hm_<id>/ws_orbit`. Callers treat the token as opaque. Display names such as `orbit-linux/ws_orbit` are not selectors.
3. The selector is addressing data, not a path, URL, logical-only workspace ID, or authorization credential. Possession of a selector is not authorization.
4. Every workspace-scoped federated tool accepts the selector. The gateway routes that call to the encoded destination.
5. **Ambiguous destination** means either (a) two configured destinations share a `machine_id`, or (b) the token is not uniquely host-qualified. Both fail as `ambiguous_destination`.

## Capabilities vs checkout roles

Capabilities distinguish at least `control_plane` and `execute`. They map onto existing host-registry catalog roles; do not invent a parallel ownership vocabulary.

| Catalog role (v1) | Advertised capabilities | Destination Core |
|---|---|---|
| Owner checkout | `control_plane`; typically also `execute` | Control-plane authority for that workspace's coordination. Refuses `execute`-class tools only when it does not advertise `execute` (for example a future control-plane-only store). |
| Replica checkout | `execute` | Execution binding. Refuses `control_plane` tools. |

1. Destination-host Core **refuses the other class** with named error `capability_refused`.
2. The gateway may advertise capabilities. It does **not** enforce by rewriting the destination.
3. If a caller routes `orbit_task_add` (or any other control-plane tool) to an execution-binding host, the destination refuses with `capability_refused`. The gateway **must not** silently send it to the owner. No implicit failover.

## Split authority ("mutations" is not one blob)

Routing delivers a call; it does not move authority.

| Concern | Authority |
|---|---|
| Runs, logs, scheduler state | **Destination host** that received the call |
| Task issuance and the coordination store | **Declared control-plane authority** for that workspace (the owner checkout today; may later be cloud-offloaded) |

Do not specify a single "mutations" permission that covers both columns. A replica may accept execute-class work against its own runs/logs/scheduler and must still refuse task issuance.

Federated `orbit_workspace_list` is an aggregate of live descriptors, not an aggregate task or store query.

## Federated `orbit_workspace_list`

1. **Session-unbound.** The tool must not require a workspace selector. Session/initialize workspace metadata is not an input and not a filter.
2. **Additive.** Each descriptor keeps today's v1 workspace fields (`id`, `name`, `ship_mode`, `owner_machine_id`, `git_remote`, `base_branch`, `status`, `created_at`, `updated_at`) and adds:

   | Field | Meaning |
   |---|---|
   | `selector` | Opaque host-qualified route token (`hm_<id>/ws_*`) |
   | `host` | Destination display identity (renameable `host_id`; display only) |
   | `machine_id` | Destination stable identity (`hm_…`) |
   | host-reachability | Whether the configured destination answers |
   | checkout-health | Repo-root presence at that destination (`active` / `invalid` / `unknown` if the host cannot be probed) |
   | `capabilities` | Classes the destination currently advertises for that workspace |

3. **Do not overload one `health` field** with SSH/MCP reachability and repo-root presence.
4. **Include unreachable hosts.** A down or unreachable configured destination appears with an explicit unreachable (and, if checkout cannot be probed, unknown/unhealthy) projection. **Do not omit it.** Omission makes every later call a stale-route surprise.
5. v1 `orbit.workspace.list` on a single accepting machine is unchanged: it remains machine-local and is documented in mcp-bridge / host-registry current-behavior docs.

## Fail-closed routing

The gateway delivers a workspace-scoped federated call to the destination encoded in the selector. It does not fall back to a local workspace, another host with a matching `ws_*`, a default workspace, or a cached host-local runtime.

| Class | Error identity | When |
|---|---|---|
| unknown | `unknown_selector` | Token does not name a configured destination+workspace |
| unreachable | `unreachable_destination` | Configured destination does not answer |
| unhealthy | `unhealthy_checkout` | Destination answers but the checkout is not usable (repo-root missing / invalid) |
| ambiguous | `ambiguous_destination` | Shared `machine_id` among destinations, or token not uniquely host-qualified |
| stale-route | `stale_route` | Encoded destination no longer advertises that workspace |
| tool not advertised | `tool_not_on_this_host` | Destination is identified; the tool is not on that host's advertised surface |
| capability refuse | `capability_refused` | Destination Core holds the workspace but refuses the tool's capability class |

`tool_not_on_this_host` is distinct from `unknown_selector`. `capability_refused` is distinct from both: the tool may be advertised elsewhere on that host, but this checkout role will not execute it.

## mcp-bridge invariant exception

The mux replaces v1 "byte-transparent / no Orbit process relays onward" **for the federated namespace only**. v1 current-behavior text in [mcp-bridge 2_design.md](../../mcp-bridge/2_design.md) stays. This spec does not claim the mux is implemented.

Still excluded (not implied by the exception):

- automatic owner discovery;
- replication of tasks or stores;
- relays-as-product (a general Orbit relay outside the federated namespace);
- fleet placement.

See [mcp-bridge 3_vision.md §5](../../mcp-bridge/3_vision.md).

## Non-goals

The following remain out of this surface:

- task or store replication;
- synchronization;
- quorum election;
- competing authorities;
- implicit failover;
- silent merging of host-local state.

A disconnected or failed host therefore removes the affected route from useful service. Another host cannot answer in its place unless a separate, explicit authority design — not this mux — says so.

## Agent Signature

Specified by grok in [ORB-11009], citing prior policy [ORB-11008].
