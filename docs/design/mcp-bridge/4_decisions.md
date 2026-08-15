---
title: Orbit MCP Bridge — Decisions
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: mcp-bridge
doc_role: decisions
type: design
summary: Current first-principles choices for the thin MCP v1 bridge.
tags: [mcp, ssh, authorization]
paths: ["crates/orbit-mcp/**", "crates/orbit-cli/src/command/mcp/**", "crates/orbit-core/src/command/tool/**"]
related_features: [mcp-bridge, mcp-session-context]
related_artifacts: []
---

# Orbit MCP Bridge — Decisions

## Use direct SSH stdio

**Context.** MCP is already a bidirectional stdio protocol and SSH already
provides authenticated process launch and byte transport.

**Decision.** Remote mode starts one `ssh -T` child whose remote command is
`orbit mcp serve`. The child inherits stdio; Orbit adds no listener or tunnel.

**Consequences.** The path has one network hop and no frame relay. Each client
gets isolated server state. The remote binary must be installed and available on
the non-interactive SSH path.

## Make the accepting machine authoritative

**Context.** Client-side checkout and ownership checks can drift from the state
that will actually execute the request.

**Decision.** The accepting server resolves its own registry and runtime and
performs validation at the execution boundary. The client only selects an SSH
destination and forwards bytes.

**Consequences.** Local and remote requests have the same authority model. A
caller cannot make a checkout authoritative by presenting local evidence.

## Treat caller identity as audit metadata

**Context.** A forwarded machine label or source IP helps correlate activity but
does not prove an Orbit principal.

**Decision.** Persist caller label and best-effort SSH IP separately from the
accepting machine's identity. Do not authorize against them in v1.

**Consequences.** Audit remains useful without pretending the transport provides
an application identity. Explicit authorization can be added later in Core.

## Keep Web tunneling separate

**Context.** `orbit web connect` forwards HTTP to a loopback listener; MCP runs
directly over stdio. They have different lifecycle and readiness requirements.

**Decision.** `orbit-web` owns its SSH local-forward implementation. `orbit-mcp`
owns direct SSH stdio. Common owns only generic POSIX argument quoting.

**Consequences.** Neither feature depends on the other, and transport-specific
code stays beside its only consumer.
