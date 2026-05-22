---
summary: "MCP Session Context — Vision"
type: design
title: "MCP Session Context — Vision"
owner: codex
last_updated: 2026-05-22
status: Draft
feature: mcp-session-context
doc_role: vision
tags: ["mcp-session-context", "mcp", "workspace"]
paths: ["crates/orbit-mcp/**", "crates/orbit-tools/**", "crates/orbit-core/src/command/tool.rs", "crates/orbit-cli/src/command/mcp/**"]
related_features: ["mcp-session-context", "task-artifacts"]
related_artifacts: ["ORB-00256", "ADR-0181", "ADR-0149"]
---

# MCP Session Context — Vision

Session context should stay small and deliberate: fields belong here only when they are session-scoped, trusted by the transport boundary, and safer than repeating low-signal inputs on every tool call.

---

## 1. Open Questions

1. Should future MCP transports store context by transport session id rather than by server instance?
2. Should workspace context eventually accept a workspace id as well as a path, or should the path remain the only MCP address and `.orbit/config.yaml` remain the binding source?
3. Which additional low-noise fields, if any, are safe enough for session context rather than tool input?

## 2. Prior Work

### Task Artifacts

[ADR-0149] established that `.orbit/config.yaml` stores the load-bearing `workspace_id` binding and that defaulting task writes from cwd can silently route to the wrong workspace.

### MCP Schema Trimming

[ORB-00255] motivated reducing repetitive tool fields, but workspace could not become optional until MCP had a deliberate ambient channel.

## 3. What May Be Distinctive

Orbit treats session context as a safety mechanism rather than a convenience cache. The resolver deliberately refuses to use process cwd, even when cwd would appear to work, because worktree and non-default binding cases are exactly where an implicit fallback is most dangerous.

## 4. References

- [ADR-0149] records the task-artifact workspace binding invariant.
- [ADR-0181] records the MCP ambient workspace session context decision.
- [ORB-00256] implemented the first session context field.
- [ORB-00255] motivated the schema trimming pressure that made a safe default useful.

## Task References

- [ORB-00255] motivated reducing repetitive workspace boilerplate.
- [ORB-00256] implemented the session context channel.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
