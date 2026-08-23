---
type: design
summary: "Glossary: Federated MCP"
last_validated: 2026-08-23
title: Glossary — Federated MCP
owner: grok
status: Draft
feature: federated-mcp
tags: [federated-mcp, glossary]
related_features: [federated-mcp, host-registry, mcp-bridge]
related_artifacts: [ORB-11009, ORB-11008]
---

# Glossary: Federated MCP

Vocabulary for the proposed federated MCP mux. Standard industry terms (proxy, mux, TTL) are included only when this feature gives them a specific meaning. Host-registry catalog terms (`machine_id`, owner checkout, replica checkout, checkout health) keep their v1 meanings; this table maps them into the federated surface rather than redefining them.

| Term | Meaning |
|------|---------|
| **Ambiguous destination** | Two configured destinations share a `machine_id`, or the selector is not uniquely host-qualified. Fails as `ambiguous_destination`. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Capabilities** | Advertised classes for a destination+workspace, at least `control_plane` and `execute`. Mapped onto owner vs replica checkouts. [2_design.md §3](../2_design.md) |
| **Checkout health** | Repo-root presence at the destination (`active` / `invalid` / `unknown` when the host cannot be probed). Not SSH reachability. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Control-plane authority** | The declared owner checkout (or a later cloud-offloaded store) that owns task issuance and the coordination store for that workspace. [2_design.md §4](../2_design.md) |
| **Destination** | An operator-configured MCP or SSH remote the mux may forward to. Not a host-registry fleet member. [2_design.md §1](../2_design.md) |
| **Execute binding** | A replica checkout: it can run, log, and schedule on that host and must refuse control-plane tools. [2_design.md §3](../2_design.md) |
| **Fail-closed routing** | Unknown, unreachable, unhealthy, ambiguous, and stale routes fail explicitly; no local fallback, default workspace, or `ws_*` substitution. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Federated namespace** | The caller-facing MCP surface that accepts host-qualified selectors and list-without-session. The only place the v1 no-relay rule is excepted. [mcp-bridge 3_vision.md §5](../../mcp-bridge/3_vision.md) |
| **Gateway** | The mux process that advertises the federated namespace. It does not own destination state and does not rewrite destinations. [1_overview.md](../1_overview.md) |
| **Host reachability** | Whether the configured destination answers (reachable / unreachable). Separate from checkout health. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Host-qualified selector** | Opaque addressing token keyed by `machine_id`, for example `hm_<id>/ws_orbit`. Not `host_id`, not a path, not a credential. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Mux** | A configured forwarder of already-chosen destinations, not a registry and not automatic owner discovery. [2_design.md §1](../2_design.md) |
| **Stale route** | The encoded destination no longer advertises that workspace. Fails as `stale_route`. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
| **Tool-not-on-this-host** | The destination is identified but does not advertise the tool. Distinct from `unknown_selector`. [specs/federated-workspace-mcp.md](../specs/federated-workspace-mcp.md) |
