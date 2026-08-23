---
summary: "Host Registry — Vision"
type: design
title: "Host Registry — Vision"
owner: codex
last_updated: 2026-08-23
last_validated: 2026-08-15
status: Accepted
feature: host-registry
doc_role: vision
tags: [host-registry, machine-identity, workspace-catalog]
paths: ["crates/orbit-common/src/types/host.rs", "crates/orbit-common/src/types/workspace.rs", "crates/orbit-registry/src/host_identity.rs", "crates/orbit-registry/src/workspace_registry/**", "crates/orbit-cmd/src/registry_runtime.rs"]
related_features: [host-registry, mcp-session-context, remote-access]
related_artifacts: [ORB-11008]
---

# Host Registry — Vision

Keep the registry small: one durable local machine identity, one validated local
workspace catalog, and one shared composition seam for opening Core runtimes.
The federated MCP surface below is a proposed design, not current v1 behavior
([ORB-11008] documents that proposal).

## Current v1 boundary

V1 exposes machine-local MCP discovery and resolves every workspace-scoped tool
on the accepting machine. Direct SSH stdio reaches a chosen remote server, but
there is no federation gateway, cross-host workspace list, or host-qualified
workspace selector. The local host registry remains neither a fleet inventory
nor a routing authority for another machine.

## Proposed federated workspace MCP surface

A future gateway may expose one Orbit MCP namespace to a caller while retaining
one MCP server and one registry at each destination host. Its one aggregate
discovery tool, `orbit_workspace_list`, returns every reachable workspace as a
live descriptor:

| Field | Meaning |
|---|---|
| `selector` | An opaque, host-qualified route token, for example `orbit-linux/ws_orbit` |
| `host` | The destination host's display identity |
| `machine_id` | The destination host's stable machine identity |
| `health` | The gateway's current reachability and advertised workspace health projection |
| `capabilities` | The operations the destination currently advertises for that workspace |

The selector is addressing data, not a path, URL, logical workspace ID, or
authorization credential. Every workspace-scoped Orbit tool accepts it and the
gateway routes that call to the encoded destination. Callers choose a workspace
without first choosing a host, but the gateway must not reinterpret the token
against its own local catalog.

Routing fails explicitly for an unknown selector, an unreachable or unhealthy
host, an ambiguous destination, or a stale route whose destination no longer
advertises that workspace. It must not fall back to a local workspace, another
host with a matching workspace ID, a default workspace, or a cached host-local
runtime.

## Authority and repository bindings

Routing changes where a call is delivered; it does not move its authority. The
destination host remains authoritative for its runs, logs, scheduler state, and
mutations. `orbit_workspace_list` is an aggregate of live descriptors, not an
aggregate task or store query.

For one repository, one declared control-plane authority owns the coordination
store. Additional checkouts on other hosts are execution bindings to that
authority. They do not become competing task stores merely because the
federated surface can route to them. The authority rule belongs to the runtime
and coordination boundary, while the gateway only preserves the selected
destination.

## Explicit non-goals

This proposal excludes task or store replication, synchronization, quorum
election, competing authorities, implicit failover, and silent merging of
host-local state. A disconnected or failed host therefore removes the affected
route from useful service; another host cannot answer in its place unless a
separate, explicit authority design says so.

## Questions that require evidence

### Old database cleanup

Remove obsolete registry tables and code only through a migration-safe change. Their historical presence does not justify reviving a fleet control plane.

### Host rename recovery

The current two-file rename is ordered but not transactional. If stale owner display names become operationally significant, add a deterministic repair/check command or one recoverable journal before expanding rename semantics.

### Legacy catalog repair

Identity-bearing legacy catalogs with no explicit checkout role fail safely but lack an automatic inference path. A migration tool is justified only if real installations still carry that shape and the intended owner can be established without guessing.

### Authenticated authorization

If Core later authorizes remote calls, it needs an authenticated principal or grant separate from machine_id, host_id, caller IP and SSH audit labels. Existing registry and session fields must not be promoted into credentials by implication.

### Federated routing contract

The gateway needs a transport-authentication and capability-advertisement
contract before it can expose live descriptors. That contract must define
health freshness, selector expiry, error identities, and how a destination
verifies that the routed selector still names its local workspace without
turning selector possession into authorization.

### Checkoutless operations

If a future operation truly needs no checkout, define a narrow server-owned API and persistence contract for it. Do not weaken RegisteredRuntimeFactory or make the SSH proxy perform placement and workspace logic.

### Schema evolution

New host or catalog schema versions need explicit forward migration, crash-safe writes, future-version rejection tests and rollback classification.

## Task References

- [ORB-11008] proposes the federated multi-host workspace MCP surface and its authority boundary.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
