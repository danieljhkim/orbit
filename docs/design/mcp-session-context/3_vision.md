---
summary: "MCP Session Context — Vision"
type: design
title: "MCP Session Context — Vision"
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-session-context
doc_role: vision
tags: ["mcp-session-context", "mcp", "workspace", "audit"]
paths: ["crates/orbit-common/src/types/tool.rs", "crates/orbit-mcp/src/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**"]
related_features: ["mcp-session-context"]
related_artifacts: []
---

# MCP Session Context — Vision

Session context should remain a small provenance and correlation envelope. Tool-domain inputs belong in tool schemas; security authority belongs in Core.

## Evolution gates

### Authorization

Future authorization belongs behind Core dispatch. It needs an authenticated principal or grant source distinct from caller_machine_id, caller_ip, hostname, and SSH target. Existing audit labels must never silently become credentials.

### Additional transports

A new transport must construct process and transport facts at the accepting server, preserve raw MCP framing, isolate mutable session state, and create one trace per call. It must not reintroduce a shared unauthenticated TCP listener.

### Additional context fields

Add a field only when all three are true:

1. a trusted Orbit boundary can derive it;
2. it has session or invocation lifetime;
3. Core dispatch or audit has a concrete consumer.

## Stable principles

- External workspace values address server state; they do not prove identity.
- The accepting machine describes itself.
- Caller machine and network labels are useful for audit correlation but remain fallible.
- The MCP adapter owns call correlation.
- Core remains the execution and audit authority.
