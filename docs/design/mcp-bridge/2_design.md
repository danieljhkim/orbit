---
title: Orbit MCP — Design
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: mcp-bridge
doc_role: design
type: design
summary: Implemented v1 request flow for local MCP stdio and direct SSH stdio, with server-side resolution and one Core audit boundary.
tags: [mcp, ssh, remote-access, registry, audit]
paths: ["crates/orbit-mcp/**", "crates/orbit-registry/**", "crates/orbit-core/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-tools/**"]
related_features: [host-registry, mcp-session-context]
---

# Orbit MCP — Design

## 1. Invariants

The v1 design rests on six invariants:

1. The accepting machine is authoritative for registry and runtime state.
2. A remote session is one direct SSH hop; no Orbit process relays it onward.
3. The client-side proxy is byte-transparent and policy-free.
4. Every tools/call enters Core's dispatch and audit boundary exactly once,
   including global discovery, unknown raw names, and setup failures.
5. Caller metadata is useful for audit correlation but grants no authority.
6. Local and remote calls use the same server implementation.

## 2. Components

### `orbit-mcp`

Owns MCP framing, advertised-name translation, structured responses, canonical
surface composition, per-call trace creation, server identity context, and the
direct SSH stdio proxy. Its `McpHost` boundary accepts canonical tool calls with a
trusted session context. It does not open an Orbit runtime or decide policy.

### `orbit-registry`

Owns persisted host identity and workspace-registry state. The accepting process
uses it to derive its own identity and resolve server-local workspaces.

### `orbit-cli`

Composes the concrete MCP host. It loads the accepting machine's registry,
selects a workspace when required, opens the corresponding runtime, and hands the
operation to Core.

### `orbit-core`

Owns domain execution, sandbox validation, and invocation audit persistence. It
is the future home of authorization because it is the single boundary reached by
local and remote calls after server-side resolution.

### `orbit-tools`

Owns canonical builtin tool definitions. `orbit-mcp` composes those definitions
with its small machine-local discovery surface rather than redeclaring schemas.

## 3. Local request flow

The client starts `orbit mcp serve` and speaks MCP over the process's stdio.

1. The server resolves the global Orbit root and its own process identity.
2. MCP initialization may establish a workspace selector for the session.
3. Tool discovery returns the canonical composed surface.
4. For each `tools/call`, the adapter creates a fresh `trace_id` and combines it
   with server-established session context. Audit fields in tool input are not
   trusted context.
5. An unknown or unadvertised raw name enters Core's global audit seam and is
   recorded as denied without opening a workspace runtime.
6. A workspace-scoped call selects `workspace` from tool input first, then from
   MCP initialization metadata. Missing selection fails clearly.
7. The server resolves that selector against its own registry and opens that
   checkout's runtime.
8. The operation and its context enter Core once; Core returns or records the
   outcome at the same audit boundary.

Global tools do not require a workspace selector, but they still enter Core once.
Their server-local registry projection is supplied through Core's in-process
dispatch seam so discovery is audited like every other MCP call.

## 4. Remote request flow

A client may register the local command:

```text
orbit mcp serve --mode remote <ssh-host>
```

That process resolves its persisted machine ID when possible, falls back to
`host/local`, and starts the equivalent of:

```text
ssh -T <ssh-host> orbit mcp serve --remote-caller-machine-id <audit-label>
```

`-T` prevents PTY allocation. The SSH child inherits all three standard streams,
so the local proxy never sees or rewrites an MCP message. SSH handles transport,
host verification, encryption, and access to the remote shell.

The remote `orbit mcp serve` process then follows the local request flow. It also
marks the session as `ssh-mcp` and reads the first field of `SSH_CONNECTION` as a
best-effort caller IP. The supplied machine label and observed IP are audit data,
not proof of caller identity.

If SSH cannot start or exits unsuccessfully, the proxy reports that transport
failure. It does not retry or replay tool calls because it cannot know whether a
request crossed the process boundary.

## 5. Workspace authority

The client selects only an SSH destination. It does not inspect a checkout or a
registry to decide where an operation belongs.

On the accepting server:

- `orbit.workspace.list` reads the machine-local registry and returns active
  logical workspaces with a checkout registered on that machine, including
  locally registered replicas;
- workspace-scoped tools accept a stable workspace selector from the call or
  initialization metadata;
- the selector is resolved to a server-local checkout;
- the resolved runtime performs the same Core validation used by non-MCP entry
  points.

No client-side ownership, placement, capability, or authorization check is part
of correctness. UI checks may improve ergonomics, but they cannot establish
server authority.

## 6. Tool discovery and dispatch

The canonical surface is assembled in `orbit-mcp` from builtin definitions in
`orbit-tools` and MCP-owned discovery definitions. Definitions are sorted and
validated before advertisement. One source therefore drives both advertised
schemas and dispatch lookup.

Each definition contains a schema and an `McpToolScope`. Scope controls only
whether the server injects and resolves a workspace selector. It does not encode
caller capability, placement, or authorization.

The concrete server classifies known tools before opening a runtime. Known global
tools use Core's global in-process seam; known workspace tools use the resolved
runtime seam. Unknown and unadvertised raw names use the global seam and return
`tool_not_found`. All three paths cross Core exactly once, so audit coverage does
not depend on recognition or outcome.

## 7. Audit envelope

| Field | Source | V1 meaning |
|---|---|---|
| `trace_id` | MCP adapter, fresh per call | Correlates one invocation |
| `caller_machine_id` | Local server identity or SSH proxy label | Audit label only |
| `caller_ip` | First field of `SSH_CONNECTION` | Best-effort network observation |
| `process_machine_id` | Accepting machine registry | Machine executing the call |
| `process_host_id` | Accepting machine registry | Host executing the call |
| `transport` | Accepting server mode | `local` or `ssh-mcp` |

The adapter prevents caller-supplied tool input from replacing trusted session
context. V1 does not treat the caller label or IP as credentials.

## 8. Authorization boundary

V1 relies on SSH access: anyone who can run the remote command can reach the MCP
surface. There is no separate machine allowlist or capability filter.

Future authorization must run in Core. A proxy or UI may not authorize a call,
because either can be bypassed by reaching the server directly. Before Core can
enforce machine-specific rules, Orbit will need an authenticated principal rather
than reusing the audit-only machine label.

## 9. Separate web transport

`orbit-web` serves the HTTP UI and owns the local-forward SSH mechanism used by
that UI. MCP does not use that listener or tunnel. Sharing application state does
not require sharing transport code.

## 10. Verification

[`references/conformance-v1.yaml`](./references/conformance-v1.yaml) maps the
contract to source and focused tests. The important behavioral gates are:

- exact tool-surface snapshot;
- protocol and production MCP round trips;
- direct SSH command construction and inherited stdio;
- server identity and SSH caller-IP parsing;
- discovery and unknown-name denial through one Core audit boundary; and
- crate dependency-direction checks.
