---
title: Orbit MCP — Decisions
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: mcp-bridge
doc_role: decisions
type: design
summary: Current decisions for the direct-SSH MCP v1, stated without historical design chains.
tags: [mcp, ssh, remote-access, registry, audit]
paths: ["crates/orbit-mcp/**", "crates/orbit-registry/**", "crates/orbit-core/**", "crates/orbit-web/**"]
related_features: [host-registry, mcp-session-context]
---

# Orbit MCP — Decisions

This file records only the decisions that define the current implementation.

## The accepting machine is authoritative

**Context.** Client-side checkout and routing logic duplicated server knowledge
and could be bypassed by calling the server directly.

**Decision.** A client selects an SSH destination. The accepting machine resolves
its own registry and runtime, performs validation through Core, and owns the audit
record. It never asks the caller to prove local checkout placement.

**Consequences.** Local and remote calls have one correctness boundary. A caller
that selects the wrong host receives a server-side error rather than being relayed.

## Remote MCP is direct SSH stdio

**Context.** MCP already supports stdio, and the supported remote environment
already provides SSH.

**Decision.** The local proxy starts one `ssh -T` process running remote
`orbit mcp serve`. The child inherits stdin, stdout, and stderr. The proxy does not
parse frames or retry calls.

**Consequences.** There is no MCP listener, port-forward tunnel, shared broker, or
third-machine relay. SSH owns transport security and shell access; Orbit owns MCP
framing only at the accepting process.

## Every tool call crosses Core once

**Context.** Audit and validation become unreliable when discovery or failure paths
bypass the normal dispatcher.

**Decision.** Every `tools/call`, including global discovery, unknown or
unadvertised raw names, and workspace setup failures, enters Core's dispatch and
audit seam exactly once with the per-call session context. Server-local
projections and pre-runtime denials use Core's global in-process dispatch hook.

**Consequences.** Successes and failures share one audit model without adding a
Core dependency on MCP or the registry crate.

## Caller metadata is audit-only in v1

**Context.** The proxy can supply a machine label and the SSH server exposes a
source IP, but neither value authenticates an Orbit machine.

**Decision.** Record the caller label, best-effort SSH caller IP, accepting process
identity, transport, and a fresh trace ID. Use `host/local` when no machine identity
is available. Do not authorize from these fields.

**Consequences.** Records support correlation now without pretending the v1
transport establishes a durable machine principal.

## Authorization will be enforced in Core

**Context.** A UI or proxy check can always be bypassed by invoking the server.

**Decision.** V1 treats SSH access as sufficient. If machine- or workspace-specific
authorization is added, enforce it in Core after server-side resolution and after
establishing an authenticated principal.

**Consequences.** V1 remains small, and future policy has one entry point shared by
local, SSH, CLI, and other callers.

## Crates follow present responsibilities

**Context.** A broad remote feature layer accumulated unrelated registry, protocol,
routing, and UI concerns.

**Decision.** Keep MCP protocol and direct SSH support in `orbit-mcp`; host and
workspace state in `orbit-registry`; domain execution and audit in `orbit-core`;
canonical builtin definitions in `orbit-tools`; and HTTP UI behavior in
`orbit-web`.

**Consequences.** Dependency direction follows the data and execution boundaries.
There is no general remote layer between MCP and Core.

## Web and MCP transports stay separate

**Context.** The HTTP UI and MCP both use SSH in some workflows but expose different
protocols and lifecycles.

**Decision.** `orbit-web` owns its local-forward tunnel. `orbit-mcp` owns its direct
stdio SSH process. Neither transport is a shared abstraction.

**Consequences.** The UI can evolve independently without turning its loopback HTTP
listener into an MCP dependency.

## Tool schemas are composed, not copied

**Context.** Repeated schema declarations drift from actual dispatch behavior.

**Decision.** Compose the advertised MCP surface from canonical builtin definitions
and MCP-owned discovery definitions. Each definition carries only its schema and
global-versus-workspace-required scope.

**Consequences.** Discovery, dispatch lookup, snapshots, and documentation describe
one surface.
