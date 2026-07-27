---
summary: "MCP Session Context — Overview"
type: design
title: "MCP Session Context — Overview"
owner: codex
last_updated: 2026-07-18
status: Accepted
feature: mcp-session-context
doc_role: overview
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-remote/src/mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ORB-10228", "ORB-10319", "ADR-0181", "ADR-0149"]
---

# MCP Session Context — Overview

MCP session context is the trusted transport envelope carried from an Orbit MCP adapter or broker to tool dispatch. External clients may announce only the legacy workspace address selector. Orbit separately carries the validated logical workspace, caller and executing-process identity, transport, the complete effective capability set, originating session, per-call correlation, and optional leased-run correlation without trusting model-authored JSON.

This document is the entry point. [2_design.md](./2_design.md) describes the live mechanism, [3_vision.md](./3_vision.md) records open questions, and [4_decisions.md](./4_decisions.md) captures the ADR log.

---

## 1. Motivation

Orbit CLI commands can resolve workspace from the user's cwd because the process runs in the user's shell. MCP servers do not have that guarantee: the server cwd is wherever the client launched `orbit mcp serve`, while the agent may be working in a canonical checkout, a nested subdirectory, or an Orbit-managed worktree.

Before [ORB-00256], every MCP call to `orbit.task.add` had to pass `workspace`. That avoided silent misroutes, but it made tool calls noisy and encouraged schema trimming before a safe default existed. Session context gives MCP a deliberate ambient channel: the client says which workspace it means, Orbit stores that for the session, and tool calls can omit `workspace` only when that deliberate signal exists.

## 2. Core Concepts

**Session context** is transport-owned metadata, not model-authored tool input. `initialize.params._meta.orbit.workspace` is the only external metadata accepted, and it remains an untrusted address selector until a local adapter/runtime validates it. Trusted `workspace_id`, machine/host identity, transport, capabilities, session/call IDs, and lease correlation are injected only at Orbit seams.

**Capability** is carried as a complete canonical `BTreeSet<McpCapability>`, never a scalar ceiling. [ORB-10228] establishes and audits that trusted set; capability-aware listing and dispatch are enforced by later broker units.

**Correlation** keeps existing authorities intact: `AuditEvent.session_id` is unchanged; `origin_session_id` is additive; and `leased_run.run_id` populates or must match canonical `job_run_id`, with only `lease_id` added to audit.

**Workspace resolution** is the tool-level rule: explicit `workspace` input wins, then session context, then a clear `missing workspace` error. Process cwd is not part of the chain.

**Binding invariant** remains owned by [ADR-0149]. The durable task binding key is still `.orbit/config.yaml`'s `workspace_id`; session context changes only how MCP calls name the intended workspace path.

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| MCP initialization parsing | `crates/orbit-mcp/src/adapter/dispatch.rs` | [ORB-00256] |
| Session metadata DTO | `crates/orbit-common/src/types/tool.rs` | [ORB-00256] |
| Trusted provenance, capabilities, and per-call correlation | common DTOs, generic MCP session framing, Remote host policy, Core runtime, audit store | [ORB-10228], [ORB-10319] |
| Runtime dispatch thread-through | `crates/orbit-core/src/command/tool.rs` | [ORB-00256] |
| MCP host and server composition | `crates/orbit-remote/src/mcp/mod.rs` | [ORB-10319] |
| Route selection, exact-checkout resolution, and placement preflight | `crates/orbit-remote/src/mcp/host.rs` | [ORB-10262], [ORB-10319] |
| Builtin explicit/session workspace-argument fallback | `crates/orbit-tools/src/builtin/orbit/mod.rs` | [ORB-00256] |

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.
- [ORB-10228] made all non-address fields trusted adapter/runtime provenance and added additive audit correlation.
- [ORB-10319] consolidated trusted broker/session policy and MCP server composition in `orbit-remote`; `orbit-mcp` remains the generic transport kernel.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
