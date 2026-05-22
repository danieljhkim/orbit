---
summary: "MCP Session Context — Design"
type: design
title: "MCP Session Context — Design"
owner: codex
last_updated: 2026-05-22
status: Draft
feature: mcp-session-context
doc_role: design
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ADR-0181", "ADR-0149"]
---

# MCP Session Context — Design

MCP session context is a narrow metadata channel from the MCP initialize request to Orbit built-in tools. Its first field is the canonical workspace path used by workspace-taking tools.

---

## 1. Initialize Metadata

Clients announce workspace with:

```json
{
  "_meta": {
    "orbit": {
      "workspace": "/absolute/path/to/repo"
    }
  }
}
```

`orbit-mcp` also accepts the compatibility key `_meta["orbit.workspace"]`. Empty strings are ignored, so a client that does not announce workspace behaves the same as an older client.

## 2. Storage And Thread-Through

`OrbitToolServer` stores a `ToolSessionContext` in an `RwLock` for the lifetime of the stdio session. Each `tools/call` snapshots that context and passes it to `McpHost::call_tool`.

The CLI `RuntimeMcpHost` forwards the context into `OrbitRuntime::execute_tool_command_dispatch_with_session_context`, which places it on `ToolContext`. Orbit built-ins read it from `ToolContext`, not from environment variables or cwd.

## 3. Workspace Resolution

`crates/orbit-tools/src/builtin/orbit/mod.rs` owns the shared resolver:

1. If the tool input has a non-empty `workspace`, use it.
2. Else if `ToolContext.session_context.workspace` is non-empty, insert that value into the input passed to the runtime host.
3. Else return a clear `missing workspace` error.

When explicit input and session context differ, Orbit logs an info-level event and honors the explicit input. This preserves an operator escape hatch while making the mismatch visible in traces.

## 4. Task Add

`orbit.task.add` advertises `workspace` as optional over the tool schema while still accepting explicit callers unchanged. The host action still receives a concrete `workspace` field because the tool wrapper resolves or rejects before dispatch.

This means existing explicit-workspace clients continue to work, while MCP clients with session context can call `orbit.task.add` without a `workspace` field.

## 5. Concerns & Honest Limitations

The session context currently covers stdio sessions; future HTTP or multi-session transports must preserve the same per-session isolation rather than promoting the value to process-global state.

The channel carries a workspace path, not a workspace id. That keeps it compatible with the existing task-add API, but callers still depend on the workspace's `.orbit/config.yaml` binding to select the durable `workspace_id` from [ADR-0149].

## Task References

- [ORB-00256] implemented the initial session context channel and workspace resolver.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
