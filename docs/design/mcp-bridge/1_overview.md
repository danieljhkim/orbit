---
title: Orbit MCP Bridge — Overview
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-bridge
doc_role: overview
type: design
summary: A thin stdio MCP server that can be reached directly over SSH and delegates execution authority to the accepting machine.
tags: [mcp, ssh, remote-access]
paths: ["crates/orbit-mcp/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**", "crates/orbit-registry/**"]
related_features: [mcp-bridge, mcp-session-context, host-registry]
related_artifacts: []
---

# Orbit MCP Bridge — Overview

Orbit has one MCP implementation and two ways to reach it:

- `orbit mcp serve` runs an MCP server over local stdio.
- `orbit mcp serve --mode remote <ssh-host>` starts `orbit mcp serve` on the
  selected machine through a direct, non-PTY SSH stdio child.

The remote path forwards MCP bytes unchanged. It has no TCP listener, local
port forward, shared broker, frame parser, checkout preflight, owner routing,
placement logic, or capability filter.

## Authority

The accepting machine is authoritative. It resolves workspace selectors against
its own registry, opens its own registered checkout, and dispatches against that
runtime. A local client and an SSH client therefore reach the same server-side
execution path.

SSH access is sufficient for v1. Caller machine and network information is
recorded for audit correlation only; it is not an authenticated Orbit identity.
Future authorization belongs in Core, where every execution outcome can be
decided and audited consistently.

## Ownership

| Concern | Owner |
|---|---|
| MCP framing, schemas, discovery, direct SSH stdio | `orbit-mcp` |
| Host identity and workspace registry | `orbit-registry` |
| Runtime construction shared by CLI and Web | `orbit-cmd` |
| Domain execution, validation, and audit | `orbit-core` |
| CLI argument parsing and server composition | `orbit-cli` |
| HTTP dashboard and its separate SSH port forward | `orbit-web` |

The exact v1 contract is pinned in
[`references/conformance-v1.yaml`](./references/conformance-v1.yaml).
