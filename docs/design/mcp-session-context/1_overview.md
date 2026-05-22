---
summary: "MCP Session Context — Overview"
type: design
title: "MCP Session Context — Overview"
owner: codex
last_updated: 2026-05-22
status: Draft
feature: mcp-session-context
doc_role: overview
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ADR-0181", "ADR-0149"]
---

# MCP Session Context — Overview

MCP session context lets an MCP client announce the canonical Orbit workspace once during session initialization so workspace-taking tools can behave more like CLI commands without falling back to unsafe process cwd inference. The first use is workspace resolution for `orbit.task.add`, preserving the workspace-id binding invariant from [ADR-0149] while removing repeated per-call boilerplate for MCP clients.

This document is the entry point. [2_design.md](./2_design.md) describes the live mechanism, [3_vision.md](./3_vision.md) records open questions, and [4_decisions.md](./4_decisions.md) captures the ADR log.

---

## 1. Motivation

Orbit CLI commands can resolve workspace from the user's cwd because the process runs in the user's shell. MCP servers do not have that guarantee: the server cwd is wherever the client launched `orbit mcp serve`, while the agent may be working in a canonical checkout, a nested subdirectory, or an Orbit-managed worktree.

Before [ORB-00256], every MCP call to `orbit.task.add` had to pass `workspace`. That avoided silent misroutes, but it made tool calls noisy and encouraged schema trimming before a safe default existed. Session context gives MCP a deliberate ambient channel: the client says which workspace it means, Orbit stores that for the session, and tool calls can omit `workspace` only when that deliberate signal exists.

## 2. Core Concepts

**Session context** is transport-owned metadata, not model-authored tool input. For MCP, it is parsed from `initialize.params._meta.orbit.workspace` and stored on `OrbitToolServer` for the stdio server session.

**Workspace resolution** is the tool-level rule: explicit `workspace` input wins, then session context, then a clear `missing workspace` error. Process cwd is not part of the chain.

**Binding invariant** remains owned by [ADR-0149]. The durable task binding key is still `.orbit/config.yaml`'s `workspace_id`; session context changes only how MCP calls name the intended workspace path.

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| MCP initialization parsing | `crates/orbit-mcp/src/adapter/dispatch.rs` | [ORB-00256] |
| Session metadata DTO | `crates/orbit-common/src/types/tool.rs` | [ORB-00256] |
| Runtime dispatch thread-through | `crates/orbit-core/src/command/tool.rs` | [ORB-00256] |
| MCP host wiring | `crates/orbit-cli/src/command/mcp/mod.rs` | [ORB-00256] |
| Tool workspace resolution | `crates/orbit-tools/src/builtin/orbit/mod.rs` | [ORB-00256] |

## Task References

- [ORB-00256] implemented MCP ambient workspace session context.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
