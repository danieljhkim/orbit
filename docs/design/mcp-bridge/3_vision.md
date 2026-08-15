---
title: Orbit MCP Bridge — Vision
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-bridge
doc_role: vision
type: design
summary: Narrow evolution gates for authorization and additional transports.
tags: [mcp, authorization, transport]
paths: ["crates/orbit-mcp/**", "crates/orbit-core/src/command/tool/**"]
related_features: [mcp-bridge, mcp-session-context]
related_artifacts: []
---

# Orbit MCP Bridge — Vision

The v1 bridge is intentionally complete at a small boundary. New machinery is
justified only by a demonstrated need that cannot fit behind the existing
accepting-server and Core seams.

## Authorization

Authorization belongs in Core, after the server has resolved the workspace and
before domain execution. It requires an authenticated principal or explicit
grant source. `caller_machine_id`, IP address, hostname, SSH alias, and possession
of an audit label are not credentials and must never silently become credentials.

## Additional transports

A future transport must preserve these invariants:

- the accepting machine derives process identity and resolves workspaces;
- one client's mutable MCP session state cannot affect another;
- the transport does not become a second dispatch or validation layer;
- Core sees one context and records one authoritative outcome;
- ingress authentication, if any, is explicit and separate from audit metadata.

HTTP, a managed daemon, or connection pooling should not be added merely to
avoid the direct SSH process cost. The added lifecycle and security surface must
buy a measured operational benefit.
