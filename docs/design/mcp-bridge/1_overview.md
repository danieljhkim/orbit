---
title: Orbit MCP — Overview
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: mcp-bridge
doc_role: overview
type: design
summary: One authoritative Orbit MCP server, reached by local stdio, a byte-transparent direct SSH stdio proxy, or a loopback-default TCP listener.
tags: [mcp, ssh, remote-access, registry, audit]
paths: ["crates/orbit-mcp/**", "crates/orbit-registry/**", "crates/orbit-core/**", "crates/orbit-cli/src/command/mcp/**"]
related_features: [host-registry, mcp-session-context, remote-access, federated-mcp]
---

# Orbit MCP — Overview

Orbit exposes the same MCP server over three transports:

```text
local:  MCP client <-> stdio <-> orbit mcp serve  <-> Orbit Core
remote: MCP client <-> stdio <-> SSH <-> orbit mcp serve <-> Orbit Core
socket: MCP client <-> TCP  <-> orbit mcp listen <-> Orbit Core
```

The remote side is intentionally just direct SSH stdio. The local proxy starts a
non-interactive SSH process whose remote command is:

```text
ssh -T <host> orbit mcp serve --remote-caller-machine-id <audit-label>
```

The proxy inherits stdin, stdout, and stderr. It does not parse MCP frames, open a
checkout, resolve a workspace, filter tools, make authorization decisions, or
forward the call through another machine.

`orbit mcp listen` is the socket form of the same server, for deployments that
need one — typically reached through an SSH tunnel. It binds loopback unless a
wider bind is asked for explicitly, because the socket authenticates no client.
It is a transport adapter only: it adds no broker, checkout preflight, placement
routing, or capability filter.

## Runtime rule

The machine accepting `orbit mcp serve` or `orbit mcp listen` is authoritative
for the call. It:

1. derives its process identity from its local registry;
2. uses definition scope to decide whether a workspace is required;
3. resolves any required workspace against its own registry and opens that
   server-local runtime;
4. sends every call, including an unknown raw name, through Orbit Core exactly
   once; and
5. records success, failure, or denial at that boundary.

This is the same rule for stdio, SSH-originated, and socket sessions. A transport
changes only how MCP bytes reach the server.

## Audit context

Each tool call carries a fresh `trace_id`. The server also records:

- `caller_machine_id`: an audit-only label supplied by the direct SSH proxy, or
  the local process identity when available;
- `caller_ip`: the first field of `SSH_CONNECTION` for an SSH session, or the
  accepted peer's address for a listener session;
- `process_machine_id` and `process_host_id`: derived by the accepting server;
- `transport`: `local` or `ssh-mcp`. A listener session is `local`, because it
  is served by the same local process with the same envelope; `caller_ip` is
  what distinguishes it.

`host/local` is the fallback machine label when no persisted identity is
available. None of these caller fields is an authenticated authorization
principal in v1.

## Ownership

| Concern | Owner |
|---|---|
| MCP framing, tool discovery, server identity context, TCP listener, direct SSH stdio proxy | `orbit-mcp` |
| Host identity and workspace-registry state | `orbit-registry` |
| Server composition and server-local runtime selection | `orbit-cli` |
| Domain validation, sandboxing, audit persistence, and future authorization | `orbit-core` |
| Canonical builtin tool definitions | `orbit-tools` |
| HTTP UI and its own local-forward SSH connection | `orbit-web` |

`orbit-web` is a separate application surface. Its HTTP tunnel is not an MCP
transport and is not reused by MCP.

## V1 boundaries

V1 deliberately has no shared broker, local checkout preflight, owner-placement
routing, capability-based tool filtering, or Orbit authorization layer. The TCP
listener is a transport only and adds none of them: reaching the socket is
sufficient to reach the surface, exactly as SSH access is sufficient to start the
remote server. If authorization is added later, it belongs in Core, after the
accepting server has established the facts needed to enforce it.

Advertised definitions contain only schema plus global-versus-workspace-required
scope. `orbit.workspace.list` is the sole global tool and reports active logical
workspaces that have a checkout registered on the accepting machine.

The executable contract and validation map live in
[`references/conformance-v1.yaml`](./references/conformance-v1.yaml). Detailed
request flow is in [`2_design.md`](./2_design.md), future work in
[`3_vision.md`](./3_vision.md), and the current decision set in
[`4_decisions.md`](./4_decisions.md).
