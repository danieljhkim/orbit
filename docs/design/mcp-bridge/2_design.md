---
title: Orbit MCP Bridge — Design
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-bridge
doc_role: design
type: design
summary: Direct SSH stdio transport into one accepting-machine MCP server and Core dispatch path.
tags: [mcp, ssh, remote-access]
paths: ["crates/orbit-mcp/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**", "crates/orbit-registry/**"]
related_features: [mcp-bridge, mcp-session-context, host-registry]
related_artifacts: []
---

# Orbit MCP Bridge — Design

## 1. Process topology

Local:

```text
MCP client <-> orbit mcp serve <-> Core
                   stdio
```

Remote:

```text
MCP client <-> ssh -T host orbit mcp serve <-> Core on host
                   inherited stdio
```

The SSH child inherits stdin, stdout, and stderr. Orbit does not inspect or
rewrite MCP frames on the client machine. One client gets one SSH child and one
server process, so no cross-client session state exists.

## 2. Server bootstrap

The accepting process:

1. resolves its global Orbit root;
2. reads its persisted host identity, using `host/local` when absent;
3. derives `caller_ip` from the first `SSH_CONNECTION` field when available;
4. creates one `ToolSessionContext` for the MCP session;
5. advertises the canonical tool definitions owned by `orbit-mcp` and
   `orbit-tools`.

For a workspace-scoped call, the server resolves the explicit tool-input
`workspace` first and MCP initialize metadata second. It validates that selector
against the accepting machine's registry and builds the matching server-local
runtime. Missing or unknown selectors fail clearly; process cwd is not an
implicit remote routing input.

## 3. Dispatch and audit

The MCP adapter creates a fresh `trace_id` for every `tools/call`. The server
sets the resolved workspace and accepting-machine identity in the context before
execution. Runtime-backed tools enter Core's dispatch boundary, which owns domain
validation and the success, denial, or failure audit row. Machine-local global
discovery reads the same authoritative registry through the accepting host.

Audit context distinguishes:

- `caller_machine_id`: opaque client-provided audit label;
- `caller_ip`: best-effort SSH connection address;
- `process_machine_id` and `process_host_id`: accepting machine identity;
- `transport`: `local` or `ssh-mcp`;
- `trace_id`: per-call correlation.

None of those caller fields grants authority.

## 4. V1 exclusions

The bridge deliberately has no:

- TCP MCP listener or local port forward;
- reusable link pool or broker;
- client-side checkout or ownership validation;
- remote placement or third-machine relay;
- capability-based filtering or authorization;
- Orbit-authenticated caller principal.

The Web dashboard's SSH local-forward tunnel is a separate HTTP concern owned by
`orbit-web`; MCP does not share it.

## 5. Failure model

- SSH launch failures and nonzero exits are returned directly to the caller.
- An unavailable remote `orbit` binary is an SSH-command failure, not an MCP
  fallback.
- Invalid workspace selection fails on the accepting server.
- Tool failures remain MCP tool errors and are audited by the execution boundary.
- No caller-side retry rewrites or replays a request.
